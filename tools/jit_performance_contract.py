"""Admit the reviewed benchmark's complete native-route receipt, not timing targets.

This is a trusted benchmark consistency check. Content hashes do not establish
truthful execution, grading authority, or arbitrary-candidate sandboxing.
"""
import hashlib
import json
import math
from pathlib import Path

from fragment_performance import compare_statistics, statistics, validate_statistics

CASES = {
    'checked_scalar': ('jit_scalar.fluxr', 'jit_scalar.rs'),
    'branch_tree': ('host_decision.flux', 'jit_branch.rs'),
    'inlined_calls': ('jit_calls.fluxr', 'jit_calls.rs'),
    'bounded_loop': ('jit_loop.fluxr', 'jit_loop.rs'),
}
ROUTES = {'rust_checked_kernel', 'rust_checked_host', 'portable', 'named_native', 'prepared_native'}
DISTRIBUTION_CONTRACT = 'nearest-rank-all-percentiles-v1'
WARM_UNIT = 'mean ns per call averaged over a batch; result disposal and checksum included'
LIFECYCLE_UNITS = {
    'source_compile': 'ns per compact source parse/check/compile',
    'cold_prepare': 'ns per fresh-module preparation; includes code publication when admitted',
    'first_call': 'ns per first invocation with fresh workspace; result assertion included',
    'cached_prepare': 'ns per cached preparation',
    'module_drop': 'ns per prepared module destruction',
}
DISTRIBUTION_FIELDS = {
    'observation_unit', 'sample_count', 'percentiles_p0_through_p100_ns',
    'p99_9_ns', 'quantile_method', 'expected_observations_above_p99_9',
}


def checked_statistics(stats, count, unit, complete):
    if not isinstance(stats, dict):
        raise ValueError('invalid JIT statistics object')
    samples = stats.get('samples_ns')
    if (not isinstance(samples, list) or len(samples) != count
            or any(type(x) not in (int, float) or not math.isfinite(x) or x <= 0 for x in samples)):
        raise ValueError('invalid JIT timing observations')
    ordered = sorted(samples)
    expected = {'p50_ns': ordered[count // 2], 'p95_ns': ordered[(count * 95 + 99) // 100 - 1],
                'minimum_ns': ordered[0], 'maximum_ns': ordered[-1]}
    if any(type(stats.get(key)) not in (int, float) or not math.isfinite(stats[key])
           or stats[key] != value for key, value in expected.items()):
        raise ValueError('JIT timing summary differs from raw samples')
    if complete:
        if stats.get('observation_unit') != unit:
            raise ValueError('JIT observation unit differs from its measured phase')
        validate_statistics(stats, count)
    elif DISTRIBUTION_FIELDS & stats.keys():
        raise ValueError('complete JIT distribution fields require their contract marker')
    return expected['p50_ns']


def validate_jit_performance(repository, raw):
    if len(raw) > 1024 * 1024:
        raise ValueError('JIT receipt exceeds bound')
    def unique(pairs):
        out = {}
        for key, value in pairs:
            if key in out:
                raise ValueError('duplicate JIT receipt field')
            out[key] = value
        return out
    def nonfinite(value):
        raise ValueError('nonfinite JIT receipt number: ' + value)
    d = json.loads(raw, object_pairs_hook=unique, parse_constant=nonfinite)
    if (d['schema'] != 'fluxr-jit-performance-v12' or d['typed_jit_feature'] is not True
            or d.get('profile') != 'strict checked signed i64; no relaxed numerical mode'
            or d['telemetry'] != 'off' or (d['os'], d['arch']) != ('linux', 'x86_64')):
        raise ValueError('wrong JIT benchmark route/profile')
    complete = 'distribution_contract' in d
    if complete and d['distribution_contract'] != DISTRIBUTION_CONTRACT:
        raise ValueError('unknown JIT distribution contract')
    rows = d['cases']
    if len(rows) != 4 or {r['name'] for r in rows} != set(CASES):
        raise ValueError('missing or duplicate JIT workload')
    summaries = []
    for row in rows:
        if (row['rounds'] != 31 or row['calls_per_round'] != 8192
                or row['validation_input_rows'] != 2048
                or row['route_counts'] != {'TypedJit': 2048}):
            raise ValueError('JIT execution count/route differs')
        for field, name in zip(('source_sha256', 'rust_reference_sha256'), CASES[row['name']]):
            path = Path(repository) / 'examples/fixtures' / name
            if hashlib.sha256(path.read_bytes()).hexdigest() != row[field]:
                raise ValueError('JIT benchmark source binding differs')
        expected_order = [[(round_index + n) % 5 for n in range(5)] for round_index in range(31)]
        if row['order_indices'] != expected_order or set(row['timings']) != ROUTES:
            raise ValueError('JIT measurement coverage/order differs')
        medians = {}
        warm_distributions = {}
        for route, stats in row['timings'].items():
            medians[route] = checked_statistics(stats, 31, WARM_UNIT, complete)
            warm_distributions[route] = statistics(stats['samples_ns'], WARM_UNIT)
        lifecycle = row.get('lifecycle')
        if not isinstance(lifecycle, dict) or set(lifecycle) != set(LIFECYCLE_UNITS):
            raise ValueError('JIT lifecycle coverage differs')
        lifecycle_distributions = {}
        for phase, stats in lifecycle.items():
            checked_statistics(stats, 11, LIFECYCLE_UNITS[phase], complete)
            lifecycle_distributions[phase] = statistics(stats['samples_ns'], LIFECYCLE_UNITS[phase])
        ratios = {
            'portable_over_prepared':medians['portable']/medians['prepared_native'],
            'prepared_over_rust_checked_host':medians['prepared_native']/medians['rust_checked_host'],
            'named_over_prepared':medians['named_native']/medians['prepared_native']}
        if any(not math.isfinite(value) or value <= 0 for value in ratios.values()):
            raise ValueError('invalid derived JIT performance ratio')
        comparisons = []
        for baseline in ('portable', 'named_native', 'rust_checked_host'):
            comparisons.append({
                'baseline_route': baseline, 'candidate_route': 'prepared_native',
                'boundary_scope': ('checked Rust host for the same sufficient-budget success domain'
                                   if baseline == 'rust_checked_host'
                                   else 'same checked results; portable versus prepared public entry'
                                   if baseline == 'portable'
                                   else 'same checked results; named versus pre-resolved public entry'),
                **compare_statistics(warm_distributions[baseline], warm_distributions['prepared_native']),
            })
        summaries.append({'name':row['name'], 'p50_ns':medians, **ratios,
            'cold_prepare_p50_ns':row['lifecycle']['cold_prepare']['p50_ns'],
            'actual_typed_jit_validation_calls':2048,
            'warm_distributions':warm_distributions, 'lifecycle_distributions':lifecycle_distributions,
            'warm_empirical_comparisons':comparisons})
    return {'status':'passed','cases':summaries,
            'distribution_contract':DISTRIBUTION_CONTRACT,
            'distribution_origin':('validated and recomputed from raw observations' if complete
                                   else 'derived from retained legacy raw observations'),
            'warm_observation_unit':'nanoseconds per call averaged over each batch of 8192 calls; 31 batches per route',
            'lifecycle_observation_unit':'nanoseconds for one operation; 11 separately prepared modules per phase',
            'scope':'Four reviewed checked-integer success-domain workloads. Warm execution ranks and p0 through p100 plus p99.9 describe batch means, not individual-call tails. Lifecycle distributions describe one phase operation per observation. Neither establishes population-tail guarantees. Rust kernels exclude host admission; the Rust host control matches only the timed sufficient-budget success domain. Consistency success is not a performance gate, universal Rust parity, .NET comparison or live-model result.'}



def main(argv=None):
    """Validate one bounded JSON report against the selected source fixtures."""
    import argparse
    import sys

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('report', type=Path, help='JSON stdout from examples/bench_jit.rs')
    parser.add_argument('--repository', type=Path, default=Path(__file__).resolve().parents[1],
                        help='fixture source root (default: this repository)')
    args = parser.parse_args(argv)
    try:
        if not args.report.is_file():
            raise ValueError('JIT report must be a regular file')
        with args.report.open('rb') as stream:
            raw = stream.read(1024 * 1024 + 1)
        summary = validate_jit_performance(args.repository, raw)
    except (OSError, ValueError, KeyError, TypeError, IndexError, OverflowError, RecursionError) as error:
        print(json.dumps({'status': 'failed', 'error': str(error)}), file=sys.stderr)
        return 1
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
