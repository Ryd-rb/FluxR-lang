"""Synthetic report/controller checks, not source performance measurements."""
from argparse import Namespace
from contextlib import ExitStack, redirect_stderr, redirect_stdout
from copy import deepcopy
from io import StringIO
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
import source_performance as sp
from eval.runner import ProcessResult


def report(samples=1024):
    return {
        'schema': 'fluxr-source-performance-v1', 'probe_sha256': '1' * 64,
        'debug_assertions': False, 'typed_jit_feature': True, 'os': 'linux', 'arch': 'x86_64', 'target_env': 'gnu',
        'settings': {'samples': samples, 'warmup': 32, 'case_order': 'fixed; phases parse, compile, print',
                     'decimal_literals': False},
        'timer_calibration': sp.fp.statistics([10] * 4096, sp.TIMER_UNIT),
        'cases': [{'name': name, 'source_bytes': 200, 'source_sha256': '2' * 64,
                   'artifact_sha256': '3' * 64, 'canonical_bytes': 100, 'canonical_sha256': '4' * 64,
                   'native_items': 64 if name.startswith('native_') else 0,
                   'checked_functions': 65 if name.startswith('checked_') and name.endswith('_64') else 1,
                   'validation': sp.VALIDATION,
                   'phases': {phase: sp.fp.statistics([100] * samples, sp.UNIT) for phase in sp.PHASES}}
                  for name in sp.CASES],
    }


def capture(samples=1024):
    return {
        'schema': 'fluxr-source-capture-v1', 'source_identity': 'synthetic-only', 'build_id': 'synthetic-build',
        'source_and_binary_unchanged': True,
        'source': {'sha256': '5' * 64, 'cargo_lock_sha256': '6' * 64, 'probe_sha256': '1' * 64, 'files': 10},
        'binary': {'sha256': '7' * 64, 'bytes': 100},
        'compiler': {'rustc': 'rustc synthetic', 'cargo': 'cargo synthetic'},
        'configuration': {'profile': 'release', 'features': ['typed-jit'], 'samples': samples,
                          'feature_observation': sp.FEATURE_SCOPE, 'environment_flags': dict.fromkeys(sp.FLAGS)},
        'hardware': {'system': 'Linux', 'machine': 'x86_64', 'release': 'synthetic',
                     'host_fingerprint': '8' * 64, 'cpu_models': ['synthetic'], 'cpu_affinity': [0]},
        'execution': {'terminal': 'exited', 'exit_code': 0, 'seconds': 1.0, 'process_started': True,
                      'detail': None, 'stdout': {'sha256': '9' * 64, 'bytes': 100},
                      'stderr': {'sha256': 'a' * 64, 'bytes': 0}},
        'report': report(samples),
    }


class ReportTests(unittest.TestCase):
    def test_complete_report_and_capture_are_consistent(self):
        value = report()
        self.assertIs(sp.validate_report(value), value)
        value = capture()
        self.assertIs(sp.validate_capture(value), value)

    def test_retained_six_case_reports_and_explicit_decimal_extension(self):
        legacy = report()
        legacy['settings'].pop('decimal_literals')
        legacy['cases'] = legacy['cases'][:len(sp.LEGACY_CASES)]
        self.assertIs(sp.validate_report(legacy), legacy)
        current = report()
        current['settings']['decimal_literals'] = True
        with self.assertRaises(ValueError):
            sp.validate_report(current)
        decimal = deepcopy(current['cases'][-2])
        decimal['name'] = 'checked_floats_decimal_64'
        current['cases'].append(decimal)
        self.assertIs(sp.validate_report(current), current)
        current['settings']['decimal_literals'] = False
        with self.assertRaises(ValueError):
            sp.validate_report(current)

    def test_missing_duplicate_reordered_case_and_missing_phase_refuse(self):
        for edit in [lambda r: r['cases'].pop(),
                     lambda r: r['cases'].__setitem__(0, deepcopy(r['cases'][1])),
                     lambda r: r['cases'].reverse(),
                     lambda r: r['cases'][0]['phases'].pop(sp.PHASES[0])]:
            value = report()
            edit(value)
            with self.assertRaises(ValueError):
                sp.validate_report(value)

    def test_raw_samples_percentiles_and_units_must_agree(self):
        for key, invalid in [('p99_9_ns', 101), ('sample_count', 1024.0),
                             ('samples_ns', [100] * 1023),
                             ('samples_ns', [100.0] * 1024),
                             ('observation_unit', 'ns per unrelated operation')]:
            value = report()
            value['cases'][0]['phases'][sp.PHASES[0]][key] = invalid
            with self.subTest(key=key), self.assertRaises(ValueError):
                sp.validate_report(value)
        value = report()
        value['timer_calibration']['observation_unit'] = sp.UNIT
        with self.assertRaises(ValueError):
            sp.validate_report(value)

    def test_debug_weak_sampling_and_target_mismatch_refuse(self):
        for edit in [lambda c: c['report'].__setitem__('debug_assertions', True),
                     lambda c: c['report'].__setitem__('target_env', ''),
                     lambda c: c['report']['settings'].__setitem__('warmup', 0),
                     lambda c: c['configuration'].__setitem__('samples', 2048),
                     lambda c: c['configuration'].__setitem__('features', []),
                     lambda c: c['hardware'].__setitem__('machine', 'aarch64'),
                     lambda c: c['source'].__setitem__('probe_sha256', 'f' * 64),
                     lambda c: c['execution'].__setitem__('terminal', 'timeout'),
                     lambda c: c['execution'].__setitem__('exit_code', False),
                     lambda c: c['execution']['stdout'].__setitem__('bytes', sp.MAX_STDOUT + 1)]:
            value = capture()
            edit(value)
            with self.assertRaises(ValueError):
                sp.validate_capture(value)

    def test_cli_nonobject_report_is_a_structured_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'invalid.json'
            path.write_text('[]')
            output, errors = StringIO(), StringIO()
            with redirect_stdout(output), redirect_stderr(errors):
                status = sp.main(['check', str(path)])
            self.assertEqual(status, 1)
            self.assertEqual(output.getvalue(), '')
            self.assertEqual(json.loads(errors.getvalue())['status'], 'failed')


class ComparisonTests(unittest.TestCase):
    def test_every_phase_retains_all_points_and_every_rank(self):
        value = capture()
        result = sp.compare(value, deepcopy(value))
        self.assertTrue(result['all_observed_percentiles_nonregressing'])
        self.assertEqual(len(result['cases']), len(sp.CASES))
        for row in result['cases']:
            self.assertEqual(len(row['phases']), 3)
            for phase in row['phases']:
                self.assertEqual(len(phase['percentiles']), 102)
                self.assertEqual(phase['full_empirical_distribution']['compared_order_statistics'], 1024)
        self.assertIn('observational only', result['qualification'])

    def test_mismatched_legacy_and_current_catalogs_cannot_truncate_comparison(self):
        before, after = capture(), capture()
        before['report']['cases'] = before['report']['cases'][:len(sp.LEGACY_CASES)]
        with self.assertRaisesRegex(ValueError, 'workload catalog'):
            sp.compare(before, after)

    def test_hidden_rank_regression_cannot_be_masked_by_integer_percentiles(self):
        before, after = capture(), capture()
        samples = list(range(1, 1025))
        before['report']['cases'][0]['phases'][sp.PHASES[0]] = sp.fp.statistics(samples, sp.UNIT)
        changed = list(samples)
        changed[400] += 1
        after['report']['cases'][0]['phases'][sp.PHASES[0]] = sp.fp.statistics(changed, sp.UNIT)
        result = sp.compare(before, after)
        phase = result['cases'][0]['phases'][0]
        self.assertTrue(phase['all_reported_points_nonregressing'])
        self.assertFalse(result['all_observed_percentiles_nonregressing'])
        self.assertEqual(phase['full_empirical_distribution']['regressed_order_statistics'], 1)

    def test_same_artifact_printing_can_change_canonical_spelling(self):
        before, after = capture(), capture()
        after['report']['cases'][1].update(canonical_bytes=80, canonical_sha256='b' * 64)
        row = sp.compare(before, after)['cases'][1]
        self.assertEqual([p['phase'] for p in row['phases']], list(sp.PHASES))
        self.assertTrue(row['canonical_output']['changed'])
        self.assertEqual(row['canonical_output']['byte_delta'], -20)
        self.assertEqual(row['canonical_output']['candidate_sha256'], 'b' * 64)

    def test_unmatched_inputs_artifacts_locks_flags_hosts_and_compilers_refuse(self):
        for edit in [lambda c: c['report']['cases'][0].__setitem__('source_sha256', 'b' * 64),
                     lambda c: c['report']['cases'][0].__setitem__('source_bytes', 201),
                     lambda c: c['report']['cases'][0].__setitem__('artifact_sha256', 'b' * 64),
                     lambda c: c['source'].__setitem__('cargo_lock_sha256', 'b' * 64),
                     lambda c: c['configuration']['environment_flags'].__setitem__('RUSTFLAGS', '-C opt-level=1'),
                     lambda c: c['hardware'].__setitem__('host_fingerprint', 'b' * 64),
                     lambda c: c['report'].__setitem__('target_env', 'musl'),
                     lambda c: c['compiler'].__setitem__('rustc', 'rustc different')]:
            after = capture()
            edit(after)
            with self.assertRaises(ValueError):
                sp.compare(capture(), after)
        with self.assertRaises(ValueError):
            sp.compare(capture(), capture(1025))


class CaptureTests(unittest.TestCase):
    def fixture(self, temporary):
        base = Path(temporary)
        source = base / 'source'
        (source / 'examples').mkdir(parents=True)
        (source / 'Cargo.lock').write_text('synthetic lock')
        (source / 'examples/bench_fragments.rs').write_text('synthetic companion probe')
        (source / 'examples/bench_source.rs').write_text('synthetic source probe')
        binary = base / 'binary'
        binary.write_bytes(b'synthetic executable; never run')
        binary.chmod(0o700)
        args = Namespace(source=source, binary=binary, output=base / 'capture.json', identity='synthetic',
                         build_id='synthetic-build', profile='release', features='typed-jit', samples=1024, timeout=2)
        raw = report()
        raw['probe_sha256'] = sp.tree_identity(source)['probe_sha256']
        return args, raw

    def mocks(self, raw, after=None, terminal='exited'):
        stack = ExitStack()
        self.addCleanup(stack.close)
        stack.enter_context(patch.object(sp.fp, 'tool_version', side_effect=lambda command: command[0] + ' synthetic'))
        stack.enter_context(patch.object(sp.fp, 'hardware', return_value=capture()['hardware']))

        def collect(command, payload, source, stdout, stderr, **options):
            self.assertEqual(command[1:4], ['-c', sp.LAUNCH, '1024'])
            self.assertEqual(payload, b'')
            self.assertEqual(options, {'timeout_seconds': 2, 'max_stdout_bytes': sp.MAX_STDOUT,
                                       'max_stderr_bytes': sp.MAX_STDERR})
            stdout.write_bytes(sp.fp.canonical(raw))
            stderr.write_bytes(b'')
            if after:
                after()
            return ProcessResult(terminal, 0 if terminal == 'exited' else -9, 0.01, True)

        return stack.enter_context(patch.object(sp, 'run_bounded', side_effect=collect))

    def test_bounded_capture_binds_source_binary_probe_and_raw_observations(self):
        with tempfile.TemporaryDirectory() as temporary:
            args, raw = self.fixture(temporary)
            collect = self.mocks(raw)
            self.assertEqual(sp.capture(args)['status'], 'consistent')
            collect.assert_called_once()
            saved = sp.validate_capture(sp.fp.read(args.output))
            self.assertEqual(saved['report'], raw)
            self.assertEqual(saved['binary'], sp.file_digest(args.binary))
            self.assertTrue((args.output.parent / saved['logs_directory'] / 'stdout').is_file())
            with self.assertRaises(ValueError):
                sp.capture(args)

    def test_actual_feature_sampling_probe_and_source_drift_refuse(self):
        for kind in ('feature', 'sampling', 'probe', 'source', 'binary'):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as temporary:
                args, raw = self.fixture(temporary)
                after = None
                if kind == 'feature':
                    args.features = ''
                elif kind == 'sampling':
                    args.samples = 2048
                elif kind == 'probe':
                    raw['probe_sha256'] = 'f' * 64
                elif kind == 'source':
                    after = lambda: (args.source / 'changed.rs').write_text('changed')
                else:
                    after = lambda: args.binary.write_bytes(b'changed')
                collect = self.mocks(raw, after)
                if kind == 'sampling':
                    # The collector assertion expects its configured sample count.
                    args.samples = 1024
                    raw = report(2048)
                    raw['probe_sha256'] = sp.tree_identity(args.source)['probe_sha256']
                    collect.side_effect = lambda command, payload, source, stdout, stderr, **kw: (
                        stdout.write_bytes(sp.fp.canonical(raw)), stderr.write_bytes(b''),
                        ProcessResult('exited', 0, 0.01, True))[-1]
                with self.assertRaises(ValueError):
                    sp.capture(args)
                self.assertFalse(args.output.exists())

    def test_limited_failed_attempt_keeps_logs_without_publishing_capture(self):
        for terminal in ('timeout', 'output_limit', 'cancelled'):
            with self.subTest(terminal=terminal), tempfile.TemporaryDirectory() as temporary:
                args, raw = self.fixture(temporary)
                self.mocks(raw, terminal=terminal)
                with self.assertRaises(ValueError):
                    sp.capture(args)
                self.assertFalse(args.output.exists())
                logs = args.output.with_name(args.output.name + '.logs')
                self.assertEqual(sp.fp.read(logs / 'execution.json')['terminal'], terminal)

    def test_inside_source_output_refuses_before_execution(self):
        with tempfile.TemporaryDirectory() as temporary:
            args, raw = self.fixture(temporary)
            args.output = args.source / 'capture.json'
            collect = self.mocks(raw)
            with self.assertRaises(ValueError):
                sp.capture(args)
            collect.assert_not_called()


if __name__ == '__main__':
    unittest.main()
