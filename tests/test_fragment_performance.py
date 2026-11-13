"""Synthetic schema/statistical regressions; these are not performance receipts."""
from argparse import Namespace
from copy import deepcopy
from contextlib import ExitStack, redirect_stderr, redirect_stdout
from io import StringIO
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
import fragment_performance as fp
from eval.runner import ProcessResult


def stats(count, value=100):
    return fp.statistics([value] * count, 'ns per synthetic invocation')


def report(public=False, common=False):
    common = common or public
    rows = []
    for name in sorted(fp.CASE_NAMES):
        route = 'Unattributed' if public else 'TypedJit' if name in fp.NATIVE_CASES else 'Portable'
        routes = ['portable', 'named'] + ([] if common else ['prepared'])
        timings = {}
        for kind in routes:
            timed_route = 'Portable' if kind == 'portable' else 'Unattributed' if kind == 'named' else route
            timings[kind] = {'individual_call': stats(1024), 'batch_mean': stats(5),
                             'timed_individual_route_counts': {timed_route: 1024}}
        lifecycle = {phase: stats(11) for phase in fp.PHASES}
        lifecycle.update({'canonical_compile_unavailable': 'public API has no canonical SourceDocument'}
                         if public else {'canonical_compile': stats(11)})
        rows.append({'name': name, 'coverage': ['synthetic_test_only'],
                     'source_format': 'compact-v2', 'source_sha256': '1' * 64,
                     'input_limits_oracle_sha256': '2' * 64, 'artifact_sha256': '3' * 64,
                     'canonical_source_sha256': None if public else '8' * 64,
                     'canonical_source_bytes': None if public else 100,
                     'artifact_bytes': 100, 'validation_rows': 64,
                     'validation_route_counts': {route: 64},
                     'native_required_on_supported_candidate': name in fp.NATIVE_CASES,
                     'timings': timings, 'lifecycle': lifecycle})
    pauses = [200_000 if index % 100 == 0 else 10 for index in range(4096)]
    means = [sum(pauses[index:index + 64]) / 64 for index in range(0, 4096, 64)]
    return {
        'schema': 'fluxr-fragment-performance-v1', 'probe_sha256': '4' * 64,
        'workload_catalog': fp.CURRENT_CATALOG,
        'os': 'linux', 'arch': 'x86_64', 'target_env': 'gnu', 'typed_jit_feature': True,
        'debug_assertions': False, 'public_baseline_adapter': public, 'telemetry': 'off',
        'numerical_profile': 'checked integers; strict binary32; unchanged logical limits',
        'settings': {'individual_samples': 1024, 'batch_rounds': 5, 'calls_per_batch': 32,
                     'common_routes': common, 'timed_routes': ['portable', 'named'] + ([] if common else ['prepared']),
                     'validation_rows': 64, 'warmup_calls_per_route': 256, 'lifecycle_samples': 11},
        'route_order': 'rotate first route by individual input index or batch round',
        'row_order': 'repeat deterministic 64-row corpus, same row for each route',
        'cases': rows,
        'calibration': {'empty_timer': stats(4096, 10),
                        'injected_pause': fp.statistics(pauses, 'ns per synthetic invocation'),
                        'calibration_batch_means': fp.statistics(means, 'mean ns per synthetic invocation'),
                        'injected_pause_ns': 200_000, 'injection_interval': 100,
                        'injected_indices': list(range(0, 4096, 100))},
    }


def execution(stdout_bytes=100):
    return {'terminal': 'exited', 'exit_code': 0, 'seconds': 0.01, 'process_started': True,
            'detail': None, 'stdout': {'sha256': '9' * 64, 'bytes': stdout_bytes},
            'stderr': {'sha256': 'a' * 64, 'bytes': 0}}


def capture(public=False, common=False):
    return {
        'schema': 'fluxr-fragment-capture-v1', 'source_identity': 'synthetic-only',
        'build_id': 'synthetic-build', 'source_and_binary_unchanged': True,
        'source': {'sha256': '5' * 64, 'cargo_lock_sha256': '6' * 64, 'probe_sha256': '4' * 64, 'files': 39},
        'binary': {'sha256': '7' * 64, 'bytes': 100}, 'compiler': {'rustc': 'rustc synthetic', 'cargo': 'cargo synthetic'},
        'configuration': {'profile': 'release', 'features': ['typed-jit'], 'environment_flags': dict.fromkeys(fp.FLAGS)},
        'hardware': {'system': 'Linux', 'machine': 'x86_64', 'release': 'synthetic',
                     'host_fingerprint': '9' * 64, 'cpu_models': ['synthetic'], 'cpu_affinity': [0]},
        'process_startup': fp.statistics([100] * 11, 'ns per bounded startup-only process spawn, collect and reap'),
        'bounded_collection': 'posix-process-groups-v1', 'execution': execution(),
        'startup_execution': [execution(22) for _ in range(11)],
        'report': report(public, common),
    }


def legacy_capture():
    value = capture()
    value['report'].pop('workload_catalog')
    value['report']['cases'] = [row for row in value['report']['cases'] if row['name'] in fp.LEGACY_CASE_NAMES]
    for key in ('bounded_collection', 'execution', 'startup_execution'):
        value.pop(key)
    value['process_startup'] = fp.statistics([100] * 11, 'ns per startup-only process spawn to exit')
    return value


def previous_capture():
    value = capture()
    value['report']['workload_catalog'] = fp.PREVIOUS_CATALOG
    value['report']['cases'] = [row for row in value['report']['cases'] if row['name'] in fp.PREVIOUS_CASE_NAMES]
    return value


class PercentileTests(unittest.TestCase):
    def test_all_points_use_nearest_rank_and_include_endpoints(self):
        result = fp.statistics(list(range(1, 1001)), 'ns per synthetic invocation')
        self.assertEqual(result['percentiles_p0_through_p100_ns'], [1] + list(range(10, 1001, 10)))
        self.assertEqual(result['p99_9_ns'], 999)
        self.assertEqual(fp.statistics([7], 'ns per invocation')['percentiles_p0_through_p100_ns'], [7] * 101)

    def test_rare_pause_appears_in_tail_and_is_smoothed_by_batch_average(self):
        samples = [100_000 if i % 100 == 0 else 10 for i in range(1000)]
        individual = fp.statistics(samples, 'ns per invocation')
        batches = [sum(samples[i:i + 100]) / 100 for i in range(0, 1000, 100)]
        batch = fp.statistics(batches, 'ns per batch mean')
        self.assertEqual(individual['p99_9_ns'], 100_000)
        self.assertLess(batch['p99_9_ns'], individual['p99_9_ns'])

    def test_median_improvement_cannot_hide_tail_regression(self):
        before = fp.statistics([100] * 1000, 'ns per invocation')
        after = fp.statistics([50] * 998 + [10_000] * 2, 'ns per invocation')
        result = fp.compare_statistics(before, after)
        self.assertEqual(len(result['percentiles']), 102)
        self.assertFalse(result['all_observed_percentiles_nonregressing'])
        self.assertFalse(result['all_observed_percentiles_improved'])
        self.assertIn('p100', result['regressed_percentiles'])
        self.assertIn('p99.9', result['regressed_percentiles'])
        self.assertEqual(result['percentiles'][50]['observation'], 'improved')

    def test_zero_is_preserved_without_infinite_ratios(self):
        value = fp.statistics([0, 0, 1], 'ns per invocation')
        result = fp.compare_statistics(value, value)
        self.assertTrue(result['all_observed_percentiles_nonregressing'])
        self.assertFalse(result['all_observed_percentiles_improved'])
        self.assertIsNone(result['percentiles'][0]['candidate_over_baseline'])

    def test_regression_between_integer_percentiles_cannot_hide(self):
        values = list(range(1, 1001))
        changed = list(values)
        changed[400] += 0.5
        result = fp.compare_statistics(fp.statistics(values, 'ns per invocation'),
                                       fp.statistics(changed, 'ns per invocation'))
        self.assertTrue(result['all_reported_points_nonregressing'])
        self.assertFalse(result['all_observed_percentiles_nonregressing'])
        self.assertEqual(result['full_empirical_distribution']['regressed_order_statistics'], 1)
        self.assertEqual(result['full_empirical_distribution']['largest_delta_rank_one_based'], 401)

    def test_rejects_invalid_numbers_and_forged_statistics(self):
        for sample in (True, -1, float('inf'), float('nan'), 10 ** 1000, '1'):
            with self.subTest(sample=str(sample)[:30]), self.assertRaises(ValueError):
                fp.statistics([sample], 'ns per invocation')
        for field, value in [('p99_9_ns', False), ('sample_count', 3.0),
                             ('percentiles_p0_through_p100_ns', [True] * 101),
                             ('expected_observations_above_p99_9', False),
                             ('observation_unit', ''), ('samples_ns', [1, 2])]:
            data = stats(3, 1)
            data[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                fp.validate_statistics(data, 3)

    def test_mismatched_observation_unit_and_sample_count_refuse(self):
        before = stats(10)
        for after in [stats(11), fp.statistics([100] * 10, 'ns per batch mean')]:
            with self.assertRaises(ValueError):
                fp.compare_statistics(before, after)


class ReportTests(unittest.TestCase):
    def setUp(self):
        self.data = report()

    def test_complete_reports_are_consistent_without_claiming_execution(self):
        self.assertIs(fp.validate_report(self.data), self.data)
        self.assertTrue(fp.validate_report(report(True))['public_baseline_adapter'])

    def test_legacy_catalog_remains_readable_and_partial_expansion_refuses(self):
        old = legacy_capture()
        self.assertEqual(len(fp.validate_capture(old)['report']['cases']), 41)
        self.assertEqual(len(fp.validate_capture(previous_capture())['report']['cases']), 55)
        self.assertEqual(len(fp.validate_report(self.data)['cases']), 60)
        for catalog in (fp.LEGACY_CATALOG, fp.PREVIOUS_CATALOG, 'unknown'):
            bad = deepcopy(self.data)
            bad['workload_catalog'] = catalog
            with self.subTest(catalog=catalog), self.assertRaises(ValueError):
                fp.validate_report(bad)
        old['report']['workload_catalog'] = fp.CURRENT_CATALOG
        with self.assertRaises(ValueError):
            fp.validate_report(old['report'])

    def test_missing_case_route_and_phase_cannot_pass(self):
        bad = deepcopy(self.data)
        bad['cases'].pop()
        with self.assertRaises(ValueError):
            fp.validate_report(bad)
        bad = deepcopy(self.data)
        del bad['cases'][0]['timings']['prepared']
        with self.assertRaises(ValueError):
            fp.validate_report(bad)
        bad = deepcopy(self.data)
        del bad['cases'][0]['lifecycle']['artifact_decode']
        with self.assertRaises(ValueError):
            fp.validate_report(bad)

    def test_feature_presence_cannot_replace_actual_native_execution(self):
        row = next(row for row in self.data['cases'] if row['name'] == 'scalar_2')
        row['validation_route_counts'] = {'Portable': 64}
        with self.assertRaises(ValueError):
            fp.validate_report(self.data)
        row['validation_route_counts'] = {'TypedJit': 64}
        row['timings']['prepared']['timed_individual_route_counts'] = {'Portable': 1024}
        with self.assertRaises(ValueError):
            fp.validate_report(self.data)

    def test_named_calls_cannot_claim_unobserved_native_route(self):
        self.data['cases'][0]['timings']['named']['timed_individual_route_counts'] = {'TypedJit': 1024}
        with self.assertRaises(ValueError):
            fp.validate_report(self.data)

    def test_nonobject_timing_route_refuses_cleanly(self):
        self.data['cases'][0]['timings']['portable'] = 1
        with self.assertRaises(ValueError):
            fp.validate_report(self.data)

    def test_input_binding_and_rare_pause_are_required(self):
        self.data['cases'][0]['input_limits_oracle_sha256'] = 'missing'
        with self.assertRaises(ValueError):
            fp.validate_report(self.data)
        data = report()
        data['calibration']['injected_pause'] = stats(4096)
        with self.assertRaises(ValueError):
            fp.validate_report(data)

    def test_duplicate_fields_nonfinite_and_size_bound_refuse(self):
        for raw in (b'{"a":1,"a":2}', b'{"a":NaN}', b'{"a":Infinity}'):
            with self.assertRaises(ValueError):
                fp.decode(raw)
        with patch.object(fp, 'MAX_BYTES', 8), self.assertRaises(ValueError):
            fp.decode(b' ' * 9)

    def test_cli_nonobject_root_returns_a_structured_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'invalid.json'
            path.write_text('[]')
            output, error = StringIO(), StringIO()
            with redirect_stdout(output), redirect_stderr(error):
                status = fp.main(['check', str(path)])
            self.assertEqual(status, 1)
            self.assertEqual(output.getvalue(), '')
            self.assertEqual(json.loads(error.getvalue())['status'], 'failed')

    def test_report_writer_enforces_read_bound_and_never_overwrites(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'capture.json'
            with patch.object(fp, 'MAX_BYTES', 8), self.assertRaises(ValueError):
                fp.write_new(path, {'long': 'value'})
            self.assertFalse(path.exists())
            fp.write_new(path, {'v': 1})
            with self.assertRaises(FileExistsError):
                fp.write_new(path, {'v': 2})
            self.assertEqual(json.loads(path.read_text()), {'v': 1})


class ComparisonTests(unittest.TestCase):
    def test_all_percentiles_are_retained_with_open_qualification(self):
        before = capture()
        after = deepcopy(before)
        after['source_identity'] = 'synthetic-candidate'
        result = fp.compare(before, after)
        self.assertEqual(len(result['cases']), len(fp.CASE_NAMES))
        self.assertTrue(result['warm_all_observed_percentiles_nonregressing'])
        self.assertIn('observational only', result['qualification'])
        self.assertEqual(len(result['cases'][0]['warm'][0]['percentiles']), 102)

    def test_unlike_workload_catalogs_are_never_compared_as_matched(self):
        old = legacy_capture()
        self.assertEqual(len(fp.compare(old, deepcopy(old))['cases']), 41)
        previous = previous_capture()
        self.assertEqual(len(fp.compare(previous, deepcopy(previous))['cases']), 55)
        with self.assertRaisesRegex(ValueError, 'workload catalog'):
            fp.compare(old, capture())
        with self.assertRaisesRegex(ValueError, 'workload catalog'):
            fp.compare(previous, capture())

    def test_previous_catalog_keeps_its_bounded_collection_requirement(self):
        value = previous_capture()
        del value['bounded_collection']
        with self.assertRaises(ValueError):
            fp.validate_capture(value)

    def test_current_capture_requires_complete_bounded_process_outcomes(self):
        for edit in [lambda c: c.pop('bounded_collection'),
                     lambda c: c['execution'].__setitem__('terminal', 'timeout'),
                     lambda c: c['execution'].__setitem__('exit_code', False),
                     lambda c: c['execution']['stdout'].__setitem__('bytes', fp.MAX_BYTES + 1),
                     lambda c: c['startup_execution'].pop(),
                     lambda c: c['startup_execution'][0].__setitem__('terminal', 'output_limit'),
                     lambda c: c['process_startup'].__setitem__('observation_unit', 'ns per unbounded startup')]:
            value = capture()
            edit(value)
            with self.assertRaises(ValueError):
                fp.validate_capture(value)

    def test_mismatched_source_inputs_hardware_and_configuration_refuse(self):
        before = capture()
        for key in ('compiler', 'configuration', 'hardware'):
            after = deepcopy(before)
            after[key]['mismatch'] = True
            with self.subTest(key=key), self.assertRaises(ValueError):
                fp.compare(before, after)
        for key in ('source_sha256', 'input_limits_oracle_sha256'):
            after = deepcopy(before)
            after['report']['cases'][0][key] = 'a' * 64
            with self.subTest(key=key), self.assertRaises(ValueError):
                fp.compare(before, after)

    def test_public_api_differences_are_explicit_and_unmatched_phases_excluded(self):
        result = fp.compare(capture(True), capture(common=True))
        row = result['cases'][0]
        self.assertEqual(row['candidate_only_routes'], [])
        self.assertEqual({entry['phase'] for entry in row['lifecycle']},
                         {'compact_compile', 'artifact_decode', 'module_drop'})
        changed = fp.compare(capture(True), capture(common=True), ('portable', 'named'))
        self.assertTrue(changed['api_boundary_changed'])

    def test_different_route_interleaving_is_not_a_matched_experiment(self):
        with self.assertRaises(ValueError):
            fp.compare(capture(True), capture())

    def test_canonical_compile_does_not_compare_unlike_spelling(self):
        before = capture()
        after = deepcopy(before)
        after['report']['cases'][0]['canonical_source_sha256'] = 'a' * 64
        row = fp.compare(before, after)['cases'][0]
        self.assertNotIn('canonical_compile', {phase['phase'] for phase in row['lifecycle']})
        self.assertEqual(row['lifecycle_not_compared'][0]['phase'], 'canonical_compile')

    def test_incomplete_metadata_and_debug_binaries_are_rejected(self):
        for key in ('compiler', 'configuration', 'hardware'):
            value = capture()
            value[key] = {'unrelated': 1}
            with self.subTest(key=key), self.assertRaises(ValueError):
                fp.validate_capture(value)
        value = capture()
        value['report']['debug_assertions'] = True
        with self.assertRaises(ValueError):
            fp.validate_capture(value)
        value = capture()
        value['configuration']['profile'] = 'debug'
        with self.assertRaises(ValueError):
            fp.validate_capture(value)

    def test_binary_probe_binding_cannot_be_silently_replaced(self):
        value = capture()
        value['source']['probe_sha256'] = 'a' * 64
        with self.assertRaises(ValueError):
            fp.validate_capture(value)

    def test_outputs_inside_source_are_refused(self):
        with self.assertRaises(ValueError):
            fp.outside(ROOT / 'benchmark-results.json', ROOT)
        with tempfile.TemporaryDirectory() as temporary:
            self.assertEqual(fp.outside(Path(temporary) / 'result.json', ROOT), Path(temporary) / 'result.json')


class NonregressionGateTests(unittest.TestCase):
    def comparison(self, measured):
        equal = fp.compare_statistics(stats(11), stats(11))
        return {
            'cases': [{'name': 'synthetic-only',
                       'warm': [{'baseline_route': 'prepared', 'candidate_route': 'prepared',
                                 'measurement': 'individual_call', **measured}],
                       'lifecycle': [{'phase': 'compact_compile', **equal}],
                       'lifecycle_not_compared': []}],
            'process_startup': equal,
            'warm_all_observed_percentiles_nonregressing': measured['all_observed_percentiles_nonregressing'],
            'lifecycle_all_observed_percentiles_nonregressing': True,
        }

    def test_gate_checks_ranks_between_reported_integer_percentiles(self):
        values = list(range(1, 1001))
        changed = list(values)
        changed[400] += 0.5
        measured = fp.compare_statistics(fp.statistics(values, 'ns per invocation'),
                                         fp.statistics(changed, 'ns per invocation'))
        self.assertTrue(measured['all_reported_points_nonregressing'])
        gate = fp.nonregression_gate(self.comparison(measured), 'warm')
        self.assertFalse(gate['passed'])
        self.assertEqual(len(gate['failed_distributions']), 1)
        self.assertEqual(gate['failed_distributions'][0]['largest_delta_rank_one_based'], 401)
        self.assertEqual(gate['failed_distributions'][0]['regressed_order_statistics'], 1)

    def test_all_scope_includes_startup_and_matched_lifecycle(self):
        comparison = self.comparison(fp.compare_statistics(stats(11), stats(11)))
        worse = fp.compare_statistics(stats(11), stats(11, 101))
        comparison['cases'][0]['lifecycle'] = [{'phase': 'compact_compile', **worse}]
        comparison['cases'][0]['lifecycle_not_compared'] = [{'phase': 'canonical_compile',
                                                           'reason': 'source spelling differs'}]
        comparison['process_startup'] = worse
        self.assertTrue(fp.nonregression_gate(comparison, 'warm')['passed'])
        gate = fp.nonregression_gate(comparison, 'all')
        self.assertFalse(gate['passed'])
        self.assertEqual(gate['compared_distributions'], 3)
        self.assertEqual({entry['phase'] for entry in gate['failed_distributions']},
                         {'compact_compile', 'process_startup'})

    def test_equal_zero_observations_pass_without_invented_ratio(self):
        zero = fp.statistics([0, 0, 1], 'ns per invocation')
        measured = fp.compare_statistics(zero, zero)
        self.assertIsNone(measured['percentiles'][0]['candidate_over_baseline'])
        self.assertTrue(fp.nonregression_gate(self.comparison(measured), 'all')['passed'])

    def test_cli_keeps_default_observational_and_saves_before_strict_failure(self):
        measured = fp.compare_statistics(fp.statistics([100] * 1000, 'ns per invocation'),
                                         fp.statistics([50] * 998 + [10000] * 2, 'ns per invocation'))
        comparison = self.comparison(measured)
        self.assertEqual(measured['percentiles'][50]['observation'], 'improved')
        for scope, expected in ((None, 0), ('warm', 2), ('all', 2)):
            with self.subTest(scope=scope), tempfile.TemporaryDirectory() as temporary:
                output = Path(temporary) / 'comparison.json'
                args = ['compare', '--baseline', 'synthetic-before', '--candidate', 'synthetic-after',
                        '--output', str(output)]
                if scope is not None:
                    args += ['--require-nonregression', scope]
                stdout, stderr = StringIO(), StringIO()
                with patch.object(fp, 'read', return_value={}), \
                        patch.object(fp, 'compare', return_value=deepcopy(comparison)), \
                        redirect_stdout(stdout), redirect_stderr(stderr):
                    status = fp.main(args)
                self.assertEqual(status, expected)
                self.assertEqual(stderr.getvalue(), '')
                saved = json.loads(output.read_text())
                self.assertEqual(saved['cases'][0]['warm'][0]['regressed_percentiles'],
                                 measured['regressed_percentiles'])
                if scope is None:
                    self.assertNotIn('empirical_nonregression_gate', saved)
                    self.assertEqual(json.loads(stdout.getvalue())['status'], 'consistent')
                else:
                    self.assertFalse(saved['empirical_nonregression_gate']['passed'])
                    self.assertEqual(json.loads(stdout.getvalue())['status'], 'nonregression_failed')

    def test_unknown_scope_refuses(self):
        with self.assertRaises(ValueError):
            fp.nonregression_gate({}, 'median')


class CaptureTests(unittest.TestCase):
    def fixture(self, temporary):
        base = Path(temporary)
        source = base / 'source'
        (source / 'examples').mkdir(parents=True)
        (source / 'Cargo.lock').write_text('synthetic lock')
        (source / 'examples/bench_fragments.rs').write_text('synthetic probe')
        binary = base / 'binary'
        binary.write_bytes(b'synthetic executable; never run by mocked tests')
        binary.chmod(0o700)
        args = Namespace(source=source, binary=binary, output=base / 'capture.json',
                         identity='synthetic-only', build_id='synthetic-build', profile='release',
                         features='typed-jit', samples=1024, rounds=5, batch_calls=32,
                         common_routes=False, timeout=2)
        raw = report()
        raw['probe_sha256'] = fp.tree_identity(source)['probe_sha256']
        return args, raw

    def metadata(self):
        stack = ExitStack()
        self.addCleanup(stack.close)
        stack.enter_context(patch.object(fp, 'tool_version', side_effect=lambda command: command[0] + ' synthetic'))
        stack.enter_context(patch.object(fp, 'hardware', return_value=capture()['hardware']))
        return stack

    def mocks(self, raw, after=None, terminal='exited', startup_terminal='exited'):
        stack = self.metadata()

        def collect(command, payload, source, stdout, stderr, **options):
            self.assertEqual(payload, b'')
            if command[-1] == '--startup':
                self.assertEqual(options, {'timeout_seconds': 20, 'max_stdout_bytes': fp.STARTUP_STDOUT,
                                           'max_stderr_bytes': fp.MAX_STDERR})
                stdout.write_bytes(b'fluxr-fragments-ready\n')
                stderr.write_bytes(b'')
                return ProcessResult(startup_terminal, 0 if startup_terminal == 'exited' else -9, 0.01, True)
            self.assertEqual(command[:3], [sys.executable, '-c', fp.LAUNCH])
            self.assertEqual(command[3:7], ['1024', '5', '32', '0'])
            self.assertEqual(options, {'timeout_seconds': 2, 'max_stdout_bytes': fp.MAX_BYTES,
                                       'max_stderr_bytes': fp.MAX_STDERR})
            stdout.write_bytes(fp.canonical(raw))
            stderr.write_bytes(b'')
            if after:
                after()
            return ProcessResult(terminal, 0 if terminal == 'exited' else -9, 0.01, True)

        return stack.enter_context(patch.object(fp, 'run_bounded', side_effect=collect))

    def test_bounded_capture_retains_raw_observations_and_all_process_outcomes(self):
        with tempfile.TemporaryDirectory() as temporary:
            args, raw = self.fixture(temporary)
            collect = self.mocks(raw)
            self.assertEqual(fp.capture(args)['status'], 'consistent')
            self.assertEqual(collect.call_count, 12)
            saved = fp.validate_capture(fp.read(args.output))
            self.assertEqual(saved['report'], raw)
            self.assertEqual(saved['binary'], fp.file_digest(args.binary))
            self.assertEqual(len(saved['startup_execution']), 11)
            logs = args.output.with_name(args.output.name + '.logs')
            self.assertEqual(fp.read(logs / 'execution.json'), saved['execution'])
            with self.assertRaises(ValueError):
                fp.capture(args)

    def test_failed_or_limited_probe_preserves_logs_without_capture(self):
        for terminal in ('timeout', 'output_limit', 'cancelled', 'unsupported_platform'):
            with self.subTest(terminal=terminal), tempfile.TemporaryDirectory() as temporary:
                args, raw = self.fixture(temporary)
                collect = self.mocks(raw, terminal=terminal)
                with self.assertRaises(ValueError):
                    fp.capture(args)
                collect.assert_called_once()
                self.assertFalse(args.output.exists())
                logs = args.output.with_name(args.output.name + '.logs')
                self.assertEqual(fp.read(logs / 'execution.json')['terminal'], terminal)

    def test_startup_failure_cannot_be_hidden_by_a_successful_fragment_probe(self):
        with tempfile.TemporaryDirectory() as temporary:
            args, raw = self.fixture(temporary)
            collect = self.mocks(raw, startup_terminal='output_limit')
            with self.assertRaises(ValueError):
                fp.capture(args)
            self.assertEqual(collect.call_count, 2)
            self.assertFalse(args.output.exists())
            logs = args.output.with_name(args.output.name + '.logs')
            self.assertEqual(fp.read(logs / 'startup-0.json')['terminal'], 'output_limit')

    def test_inside_source_outputs_and_reused_partial_log_directory_refuse(self):
        with tempfile.TemporaryDirectory() as temporary:
            args, raw = self.fixture(temporary)
            collect = self.mocks(raw)
            output = args.output
            args.output = args.source / 'capture.json'
            with self.assertRaises(ValueError):
                fp.capture(args)
            args.output = output
            args.output.with_name(args.output.name + '.logs').mkdir()
            with self.assertRaises(FileExistsError):
                fp.capture(args)
            collect.assert_not_called()

    def test_actual_sampling_probe_feature_and_source_identity_are_required(self):
        for kind in ('sampling', 'probe', 'features', 'source', 'binary'):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as temporary:
                args, raw = self.fixture(temporary)
                after = None
                if kind == 'sampling':
                    raw['settings']['calls_per_batch'] = 64
                elif kind == 'probe':
                    raw['probe_sha256'] = 'f' * 64
                elif kind == 'features':
                    args.features = ''
                elif kind == 'source':
                    after = lambda: (args.source / 'changed.rs').write_text('changed')
                else:
                    after = lambda: args.binary.write_bytes(b'changed')
                self.mocks(raw, after)
                with self.assertRaises(ValueError):
                    fp.capture(args)
                self.assertFalse(args.output.exists())

    @unittest.skipUnless(os.name == 'posix', 'bounded collector requires POSIX')
    def test_actual_probe_output_overflow_is_bounded_and_retained(self):
        with tempfile.TemporaryDirectory() as temporary:
            args, _ = self.fixture(temporary)
            args.binary.write_text('#!' + sys.executable + '\nimport os\nwhile True: os.write(1,b"x"*4096)\n')
            self.metadata()
            with patch.object(fp, 'MAX_BYTES', 2048), self.assertRaisesRegex(ValueError, 'output_limit'):
                fp.capture(args)
            logs = args.output.with_name(args.output.name + '.logs')
            self.assertEqual((logs / 'stdout').stat().st_size, 2048)
            self.assertEqual(fp.read(logs / 'execution.json')['terminal'], 'output_limit')
            self.assertFalse(args.output.exists())

    @unittest.skipUnless(sys.platform == 'linux', 'descendant liveness check uses Linux procfs')
    def test_actual_timeout_stops_a_child_retaining_the_output_pipe(self):
        with tempfile.TemporaryDirectory() as temporary:
            args, _ = self.fixture(temporary)
            child_file = Path(temporary) / 'child.pid'
            args.binary.write_text('#!' + sys.executable + '\nimport json,os,time\n'
                                   'pid=os.fork()\nif pid==0: time.sleep(30)\n'
                                   'else:\n'
                                   ' with open("/proc/"+str(pid)+"/stat") as stream:\n'
                                   '  starttime=stream.read().rsplit(") ",1)[1].split()[19]\n'
                                   ' with open(' + repr(str(child_file)) + ',"w") as stream:\n'
                                   '  json.dump([pid,starttime],stream)\n')
            args.timeout = 1
            self.metadata()
            with self.assertRaisesRegex(ValueError, 'timeout'):
                fp.capture(args)
            pid, starttime = json.loads(child_file.read_text())
            status = Path(f'/proc/{pid}/stat')
            # Group SIGKILL is asynchronous: the leader can already be reaped
            # while this child briefly remains runnable with a pending kill.
            # Observe the original generation exiting, without sending another
            # signal that could conceal broken collector cleanup. Its 30-second
            # sleep cannot expire naturally inside this bounded two-second wait.
            deadline = time.monotonic() + 2
            while True:
                try:
                    fields = status.read_text().rsplit(') ', 1)[1].split()
                except FileNotFoundError:
                    break
                if fields[19] != starttime or fields[0] == 'Z':
                    break
                if time.monotonic() >= deadline:
                    self.fail('timed-out descendant generation remained live: ' + fields[0])
                time.sleep(0.01)
            logs = args.output.with_name(args.output.name + '.logs')
            self.assertEqual(fp.read(logs / 'execution.json')['terminal'], 'timeout')
            self.assertFalse(args.output.exists())


if __name__ == '__main__':
    unittest.main()
