"""Adversarial receipt consistency tests; synthetic rows are never runtime proof."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
from jit_performance_contract import (CASES, ROUTES, DISTRIBUTION_CONTRACT,
                                      WARM_UNIT, LIFECYCLE_UNITS, validate_jit_performance)
from fragment_performance import statistics as complete_statistics


def statistics(count):
    a=list(range(1,count+1))
    return {'samples_ns':a,'p50_ns':a[count//2], 'p95_ns':a[(count*95+99)//100-1],
            'minimum_ns':a[0],'maximum_ns':a[-1]}


class PerformanceReceiptTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.root=Path(self.temp.name)
        fixture=self.root/'examples/fixtures';fixture.mkdir(parents=True)
        rows=[]
        for case,names in CASES.items():
            hashes=[]
            for name in names:
                b=('synthetic fixture '+name).encode();(fixture/name).write_bytes(b)
                hashes.append(hashlib.sha256(b).hexdigest())
            rows.append({'name':case,'source_sha256':hashes[0],'rust_reference_sha256':hashes[1],
                'rounds':31,'calls_per_round':8192,'validation_input_rows':2048,
                'route_counts':{'TypedJit':2048},
                'order_indices':[[(i+n)%5 for n in range(5)] for i in range(31)],
                'timings':{route:statistics(31) for route in ROUTES},
                'lifecycle':{name:statistics(11) for name in ('source_compile','cold_prepare','first_call','cached_prepare','module_drop')}})
        self.payload={'schema':'fluxr-jit-performance-v12','typed_jit_feature':True,
            'profile':'strict checked signed i64; no relaxed numerical mode',
            'telemetry':'off','os':'linux','arch':'x86_64','cases':rows}

    def tearDown(self):
        self.temp.cleanup()

    def call(self,payload=None):
        return validate_jit_performance(self.root,json.dumps(payload or self.payload).encode())

    def complete_payload(self):
        payload = copy.deepcopy(self.payload)
        payload['distribution_contract'] = DISTRIBUTION_CONTRACT
        for row in payload['cases']:
            for stats in row['timings'].values():
                stats.update(complete_statistics(stats['samples_ns'], WARM_UNIT))
            for phase, stats in row['lifecycle'].items():
                stats.update(complete_statistics(stats['samples_ns'], LIFECYCLE_UNITS[phase]))
        return payload

    def test_consistent_synthetic_receipt(self):
        self.assertEqual(self.call()['status'],'passed')

    def test_legacy_raw_reports_gain_complete_empirical_distributions(self):
        result = self.call()
        self.assertEqual(result['distribution_origin'], 'derived from retained legacy raw observations')
        for row in result['cases']:
            measured = row['warm_distributions']['prepared_native']
            self.assertEqual(measured['samples_ns'], list(range(1, 32)))
            self.assertEqual(measured['sample_count'], 31)
            self.assertEqual(len(measured['percentiles_p0_through_p100_ns']), 101)
            self.assertEqual(measured['percentiles_p0_through_p100_ns'][0], 1)
            self.assertEqual(measured['percentiles_p0_through_p100_ns'][50], 16)
            self.assertEqual(measured['percentiles_p0_through_p100_ns'][100], 31)
            self.assertEqual(measured['p99_9_ns'], 31)
            self.assertEqual(set(row['lifecycle_distributions']), set(LIFECYCLE_UNITS))
            for comparison in row['warm_empirical_comparisons']:
                self.assertEqual(comparison['full_empirical_distribution']['compared_order_statistics'], 31)
                self.assertTrue(comparison['all_observed_percentiles_nonregressing'])
                self.assertEqual(len(comparison['percentiles']), 102)

    def test_complete_contract_is_additive_and_recomputable(self):
        legacy = self.call()
        complete = self.call(self.complete_payload())
        self.assertEqual(complete['distribution_contract'], DISTRIBUTION_CONTRACT)
        self.assertEqual(complete['distribution_origin'], 'validated and recomputed from raw observations')
        self.assertEqual(complete['cases'], legacy['cases'])

    def test_complete_distribution_fields_and_units_cannot_be_forged(self):
        mutations = (
            lambda row: row['timings']['prepared_native'].__setitem__('p99_9_ns', 30),
            lambda row: row['timings']['prepared_native']['percentiles_p0_through_p100_ns'].__setitem__(73, 1),
            lambda row: row['timings']['prepared_native'].__setitem__('sample_count', True),
            lambda row: row['timings']['prepared_native'].pop('quantile_method'),
            lambda row: row['timings']['prepared_native'].__setitem__('observation_unit', 'ns per invocation'),
            lambda row: row['lifecycle']['first_call'].__setitem__('observation_unit', WARM_UNIT),
            lambda row: row['lifecycle']['module_drop'].__setitem__('p99_9_ns', 10),
        )
        for index, mutate in enumerate(mutations):
            payload = self.complete_payload()
            mutate(payload['cases'][0])
            with self.subTest(mutation=index), self.assertRaises(ValueError):
                self.call(payload)
        for marker in (None, 'unknown-contract'):
            payload = self.complete_payload()
            if marker is None:
                del payload['distribution_contract']
            else:
                payload['distribution_contract'] = marker
            with self.subTest(marker=marker), self.assertRaises(ValueError):
                self.call(payload)

    def test_better_median_cannot_hide_rust_control_tail_regression(self):
        payload = copy.deepcopy(self.payload)
        row = payload['cases'][0]
        row['timings']['rust_checked_host'] = {
            'samples_ns': [100] * 31, 'p50_ns': 100, 'p95_ns': 100,
            'minimum_ns': 100, 'maximum_ns': 100,
        }
        row['timings']['prepared_native'] = {
            'samples_ns': [50] * 30 + [10_000], 'p50_ns': 50, 'p95_ns': 50,
            'minimum_ns': 50, 'maximum_ns': 10_000,
        }
        result = self.call(payload)
        comparison = next(item for item in result['cases'][0]['warm_empirical_comparisons']
                          if item['baseline_route'] == 'rust_checked_host')
        self.assertFalse(comparison['all_observed_percentiles_nonregressing'])
        self.assertEqual(comparison['full_empirical_distribution']['regressed_order_statistics'], 1)
        self.assertEqual(comparison['percentiles'][50]['observation'], 'improved')
        self.assertIn('p100', comparison['regressed_percentiles'])
        self.assertIn('p99.9', comparison['regressed_percentiles'])

    def test_wrong_execution_route_never_passes(self):
        for routes in ({'Portable':2048},{'TypedJit':2047,'PrecompiledScan':1},{'TypedJit':2049}):
            with self.subTest(routes=routes):
                d=copy.deepcopy(self.payload);d['cases'][0]['route_counts']=routes
                with self.assertRaises(ValueError):self.call(d)

    def test_missing_or_duplicated_workload(self):
        for rows in (self.payload['cases'][:3],[self.payload['cases'][0]]*4):
            d=copy.deepcopy(self.payload);d['cases']=rows
            with self.assertRaises(ValueError):self.call(d)

    def test_every_warm_observation_and_recomputed_summary_required(self):
        for key,value in [('p50_ns',1),('p95_ns',31),('minimum_ns',0),('maximum_ns',32),('samples_ns',[1]*30)]:
            d=copy.deepcopy(self.payload);d['cases'][0]['timings']['prepared_native'][key]=value
            with self.subTest(key=key),self.assertRaises(ValueError):self.call(d)

    def test_every_lifecycle_observation_and_summary_required(self):
        for phase in self.payload['cases'][0]['lifecycle']:
            for key,value in [('samples_ns',[1]*10),('p50_ns',0),('p95_ns',10),('maximum_ns',12)]:
                d=copy.deepcopy(self.payload);d['cases'][0]['lifecycle'][phase][key]=value
                with self.subTest(phase=phase,key=key),self.assertRaises(ValueError):self.call(d)
        d=copy.deepcopy(self.payload);del d['cases'][0]['lifecycle']['module_drop']
        with self.assertRaises(ValueError):self.call(d)

    def test_nonfinite_boolean_and_negative_samples_refuse(self):
        for value in (float('nan'),float('inf'),-1,True):
            for field in ('timings','lifecycle'):
                d=copy.deepcopy(self.payload);name=next(iter(d['cases'][0][field]));d['cases'][0][field][name]['samples_ns'][0]=value
                with self.subTest(value=value,field=field),self.assertRaises(ValueError):self.call(d)

    def test_source_binding_and_order_are_checked(self):
        d=copy.deepcopy(self.payload);d['cases'][0]['source_sha256']='0'*64
        with self.assertRaises(ValueError):self.call(d)
        d=copy.deepcopy(self.payload);d['cases'][0]['order_indices'][0]=[0]*5
        with self.assertRaises(ValueError):self.call(d)

    def test_duplicate_json_key_and_size_bound_refuse(self):
        raw=json.dumps(self.payload).encode().replace(b'{',b'{"schema":"wrong",',1)
        with self.assertRaises(ValueError):validate_jit_performance(self.root,raw)
        with self.assertRaises(ValueError):validate_jit_performance(self.root,b' '* (1024*1024+1))

    def test_missing_or_wrong_numerical_profile_refuses(self):
        for profile in (None, 'approximate', ''):
            d=copy.deepcopy(self.payload)
            if profile is None:del d['profile']
            else:d['profile']=profile
            with self.subTest(profile=profile),self.assertRaises(ValueError):self.call(d)

    def test_nonfinite_or_underflowed_derived_ratio_refuses(self):
        for large,small in ((1e308,1e-308),(1e-308,1e308)):
            d=copy.deepcopy(self.payload)
            for route,value in (('portable',large),('prepared_native',small)):
                d['cases'][0]['timings'][route]={'samples_ns':[value]*31,
                    'p50_ns':value,'p95_ns':value,'minimum_ns':value,'maximum_ns':value}
            with self.assertRaises(ValueError):self.call(d)


    def test_cli_accepts_explicit_repository_and_default_fixture_root(self):
        for explicit in (True, False):
            payload = copy.deepcopy(self.payload)
            repository = self.root if explicit else ROOT
            for row in payload['cases']:
                for field, name in zip(('source_sha256', 'rust_reference_sha256'), CASES[row['name']]):
                    row[field] = hashlib.sha256((repository / 'examples/fixtures' / name).read_bytes()).hexdigest()
            report = self.root / 'synthetic-report.json'
            report.write_text(json.dumps(payload))
            command = [sys.executable, '-B', str(ROOT / 'tools/jit_performance_contract.py'), str(report)]
            if explicit:
                command.extend(['--repository', str(repository)])
            with self.subTest(explicit_repository=explicit):
                result = subprocess.run(command, cwd=self.root, capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 0, result.stderr)
                summary = json.loads(result.stdout)
                self.assertEqual(summary['status'], 'passed')
                self.assertEqual(len(summary['cases']), 4)
                self.assertEqual(result.stderr, '')

    def test_cli_reports_malformed_and_oversized_input_without_traceback(self):
        report = self.root / 'invalid-report.json'
        for raw in (b'{}', b'not JSON', b' ' * (1024 * 1024 + 1)):
            report.write_bytes(raw)
            with self.subTest(bytes=len(raw)):
                result = subprocess.run(
                    [sys.executable, '-B', str(ROOT / 'tools/jit_performance_contract.py'), str(report)],
                    cwd=self.root, capture_output=True, text=True, timeout=10,
                )
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, '')
                self.assertEqual(json.loads(result.stderr)['status'], 'failed')
                self.assertNotIn('Traceback', result.stderr)


if __name__=='__main__':unittest.main()
