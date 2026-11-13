#!/usr/bin/env python3
"""Capture and compare finite checked-fragment latency distributions.

Keep every integer percentile and p99.9 visible, including regressions. Report
consistency is neither authenticated execution nor a statistical improvement
proof. Run capture under the resource coordinator with external output storage.
Only the explicitly selected, already-built local benchmark binary is executed.
"""
from __future__ import annotations

import argparse
from dataclasses import asdict
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import subprocess
import sys
import time

from eval.runner import file_digest, run_bounded

ROOT = Path(__file__).resolve().parents[1]
MAX_BYTES = 64 * 1024 * 1024
MAX_STDERR = 64 * 1024
STARTUP_STDOUT = 1024
CURRENT_CATALOG = 'checked-fragments-v3'
PREVIOUS_CATALOG = 'checked-fragments-v2'
LEGACY_CATALOG = 'checked-fragments-v1'
LEGACY_CASE_NAMES = {
    'scalar_0', 'scalar_1', 'scalar_2', 'scalar_3', 'scalar_4', 'scalar_5',
    'split_inputs', 'branch', 'boolean_short_circuit', 'bounded_loop',
    'loop_exits', 'acyclic_calls', 'recursive_calls', 'record_read',
    'record_update', 'record_construct', 'nested_record', 'method', 'unsigned',
    'binary32', 'float_conversions', 'list_index', 'list_copy', 'nested_list',
    'variant_payload', 'binary32_scan', 'scan_empty', 'scan_short', 'scan_long',
    'scan_descending', 'scan_mixed',
    'unit', 'overflow', 'division_trap', 'uninitialized_trap', 'index_trap',
    'fuel_limit', 'cell_limit', 'frame_limit', 'wrong_type', 'wrong_arity',
}
LEGACY_NATIVE_CASES = {
    'scalar_0', 'scalar_1', 'scalar_2', 'scalar_3', 'scalar_4', 'scalar_5',
    'split_inputs', 'branch', 'bounded_loop', 'acyclic_calls',
    'record_read', 'record_update',
}
ADDED_NATIVE_CASES = {
    'scalar_compare_bool', 'scalar_bool_identity', 'scalar_bool_equal',
    'scalar_mixed_branch', 'scalar_split_bool', 'scalar_mixed_5',
}
PREVIOUS_CASE_NAMES = LEGACY_CASE_NAMES | ADDED_NATIVE_CASES | {
    'list_construct', 'list_empty', 'last_empty_trap', 'pop_empty_trap',
    'variant_payload_trap', 'float_nonfinite_to_int', 'float_range_to_int', 'unsigned_overflow',
}
LOOP_NATIVE_CASES = {'loop_nested', 'loop_branch', 'loop_acyclic_calls',
                     'looping_callee', 'loop_record_update'}
CASE_NAMES = PREVIOUS_CASE_NAMES | LOOP_NATIVE_CASES
PREVIOUS_NATIVE_CASES = LEGACY_NATIVE_CASES | ADDED_NATIVE_CASES
NATIVE_CASES = PREVIOUS_NATIVE_CASES | LOOP_NATIVE_CASES
CATALOGS = {
    LEGACY_CATALOG: (LEGACY_CASE_NAMES, LEGACY_NATIVE_CASES),
    PREVIOUS_CATALOG: (PREVIOUS_CASE_NAMES, PREVIOUS_NATIVE_CASES),
    CURRENT_CATALOG: (CASE_NAMES, NATIVE_CASES),
}
PHASES = {'compact_compile', 'artifact_decode', 'cold_prepare', 'first_call',
          'cached_prepare', 'module_drop'}
FLAGS = ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_INCREMENTAL',
         'CARGO_PROFILE_RELEASE_OPT_LEVEL', 'CARGO_PROFILE_RELEASE_LTO',
         'CARGO_PROFILE_RELEASE_CODEGEN_UNITS', 'CARGO_PROFILE_RELEASE_DEBUG',
         'CARGO_PROFILE_RELEASE_PANIC', 'CARGO_PROFILE_RELEASE_STRIP')
# execv preserves the collector's process group. Only the four experiment knobs
# are changed, and Python startup is outside every in-probe timing interval.
LAUNCH = ("import os,sys; "
          "os.environ.update(zip(('FLUXR_FRAGMENT_SAMPLES','FLUXR_FRAGMENT_ROUNDS',"
          "'FLUXR_FRAGMENT_BATCH_CALLS','FLUXR_FRAGMENT_COMMON_ROUTES'),sys.argv[1:5])); "
          "os.execv(sys.argv[5],[sys.argv[5]])")


def workload_catalog(report):
    catalog = report.get('workload_catalog', LEGACY_CATALOG)
    if catalog not in CATALOGS:
        raise ValueError('unknown fragment workload catalog')
    return catalog


def sha(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()


def write_new(path, value):
    payload = canonical(value) + b'\n'
    if len(payload) > MAX_BYTES:
        raise ValueError('serialized report exceeds its own 64 MiB read bound')
    with Path(path).open('xb') as stream:
        stream.write(payload)


def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate JSON field: ' + key)
        result[key] = value
    return result


def decode(raw):
    if len(raw) > MAX_BYTES:
        raise ValueError('fragment report exceeds 64 MiB')

    def nonfinite(value):
        raise ValueError('nonfinite JSON number: ' + value)

    return json.loads(raw, object_pairs_hook=unique, parse_constant=nonfinite)


def read(path):
    with Path(path).open('rb') as stream:
        return decode(stream.read(MAX_BYTES + 1))


def number(value):
    try:
        return type(value) in (int, float) and math.isfinite(value) and value >= 0
    except OverflowError:
        return False


def integer(value, minimum, maximum):
    return type(value) is int and minimum <= value <= maximum


def digest_valid(value):
    return (isinstance(value, str) and len(value) == 64
            and all(c in '0123456789abcdef' for c in value))


def statistics(samples, unit):
    """One nearest-rank convention for every point, including both endpoints."""
    if (not isinstance(samples, list) or not samples or len(samples) > 16_384
            or not all(number(value) for value in samples)):
        raise ValueError('invalid nonnegative timing samples')
    ordered = sorted(samples)

    def at(tenths):
        return ordered[max(1, (len(ordered) * tenths + 999) // 1000) - 1]

    return {
        'observation_unit': unit, 'sample_count': len(samples), 'samples_ns': samples,
        'percentiles_p0_through_p100_ns': [at(p * 10) for p in range(101)],
        'p99_9_ns': at(999), 'quantile_method': 'nearest-rank; p0 is minimum',
        'expected_observations_above_p99_9': len(samples) / 1000,
    }


def validate_statistics(value, count):
    if not isinstance(value, dict) or not isinstance(value.get('observation_unit'), str):
        raise ValueError('statistics require an observation unit')
    if not value['observation_unit'].startswith(('ns per ', 'mean ns per ')):
        raise ValueError('statistics have an unrecognized observation unit')
    samples = value.get('samples_ns')
    if not isinstance(samples, list) or len(samples) != count:
        raise ValueError('timing sample count differs')
    expected = statistics(samples, value['observation_unit'])
    points = value.get('percentiles_p0_through_p100_ns')
    if (not isinstance(points, list) or len(points) != 101 or not all(number(x) for x in points)
            or not number(value.get('p99_9_ns'))
            or not number(value.get('expected_observations_above_p99_9'))):
        raise ValueError('invalid percentile summary')
    for key, result in expected.items():
        if value.get(key) != result:
            raise ValueError('timing summary differs from observations: ' + key)
    if type(value.get('sample_count')) is not int:
        raise ValueError('sample_count must be an integer')
    return expected


def validate_counts(counts, count, allowed):
    if (not isinstance(counts, dict) or not counts or not set(counts) <= allowed
            or any(not integer(n, 1, count) for n in counts.values())
            or sum(counts.values()) != count):
        raise ValueError('execution route counts differ')


def validate_report(report):
    if (not isinstance(report, dict) or report.get('schema') != 'fluxr-fragment-performance-v1'
            or report.get('telemetry') != 'off'
            or report.get('numerical_profile') != 'checked integers; strict binary32; unchanged logical limits'
            or not digest_valid(report.get('probe_sha256'))):
        raise ValueError('wrong fragment benchmark identity or numerical profile')
    for key in ('typed_jit_feature', 'debug_assertions', 'public_baseline_adapter'):
        if type(report.get(key)) is not bool:
            raise ValueError('invalid build attribute: ' + key)
    if not all(isinstance(report.get(key), str) and report[key] for key in ('os', 'arch', 'target_env')):
        raise ValueError('missing target identity')
    settings = report.get('settings')
    if (not isinstance(settings, dict)
            or not integer(settings.get('individual_samples'), 1024, 16_384)
            or not integer(settings.get('batch_rounds'), 5, 101)
            or not integer(settings.get('calls_per_batch'), 32, 4096)
            or settings.get('validation_rows') != 64
            or settings.get('warmup_calls_per_route') != 256
            or settings.get('lifecycle_samples') != 11
            or type(settings.get('common_routes')) is not bool
            or settings.get('timed_routes') != ['portable', 'named'] + ([] if settings.get('common_routes') else ['prepared'])):
        raise ValueError('invalid or weakened experiment settings')
    if (report.get('route_order') != 'rotate first route by individual input index or batch round'
            or report.get('row_order') != 'repeat deterministic 64-row corpus, same row for each route'):
        raise ValueError('experiment order differs')
    catalog = workload_catalog(report)
    case_names, native_cases = CATALOGS[catalog]
    cases = report.get('cases')
    if (not isinstance(cases, list) or len(cases) != len(case_names)
            or {row.get('name') for row in cases if isinstance(row, dict)} != case_names):
        raise ValueError('missing, unknown or duplicate fragment workload')
    public = report['public_baseline_adapter']
    if public and not settings['common_routes']:
        raise ValueError('public API adapter requires the common two-route schedule')
    native_required = (not public and report['typed_jit_feature']
                       and (report['os'], report['arch'], report['target_env']) == ('linux', 'x86_64', 'gnu'))
    routes = set(settings['timed_routes'])
    for row in cases:
        name = row['name']
        if (row.get('source_format') != 'compact-v2' or row.get('validation_rows') != 64
                or not row.get('coverage') or not isinstance(row['coverage'], list)
                or not all(isinstance(label, str) and label for label in row['coverage'])
                or type(row.get('native_required_on_supported_candidate')) is not bool
                or row['native_required_on_supported_candidate'] != (name in native_cases)):
            raise ValueError('fragment coverage or native contract differs: ' + name)
        for field in ('source_sha256', 'input_limits_oracle_sha256', 'artifact_sha256'):
            if not digest_valid(row.get(field)):
                raise ValueError('missing source/input/artifact binding: ' + name)
        if public:
            if row.get('canonical_source_sha256') is not None or row.get('canonical_source_bytes') is not None:
                raise ValueError('public adapter cannot claim canonical source support')
        elif (not digest_valid(row.get('canonical_source_sha256'))
                or not integer(row.get('canonical_source_bytes'), 1, 8 * 1024 * 1024)):
            raise ValueError('missing canonical source identity')
        if not integer(row.get('artifact_bytes'), 1, 16 * 1024 * 1024):
            raise ValueError('invalid artifact size')
        witnesses = {'Unattributed'} if public else {'Portable', 'TypedJit', 'NativeScalar', 'PrecompiledScan', 'AheadOfTime'}
        validate_counts(row.get('validation_route_counts'), 64, witnesses)
        if native_required and name in native_cases and row['validation_route_counts'] != {'TypedJit': 64}:
            raise ValueError('required validation JIT route missing: ' + name)
        timings = row.get('timings')
        if not isinstance(timings, dict) or set(timings) != routes:
            raise ValueError('missing warm execution route: ' + name)
        for route, times in timings.items():
            if not isinstance(times, dict):
                raise ValueError('timing route must be an object')
            validate_statistics(times.get('individual_call'), settings['individual_samples'])
            validate_statistics(times.get('batch_mean'), settings['batch_rounds'])
            allowed = {'Portable'} if route == 'portable' else {'Unattributed'} if route == 'named' else witnesses
            validate_counts(times.get('timed_individual_route_counts'), settings['individual_samples'], allowed)
            if (route == 'prepared' and native_required and name in native_cases
                    and times['timed_individual_route_counts'] != {'TypedJit': settings['individual_samples']}):
                raise ValueError('required timed JIT route missing: ' + name)
        lifecycle = row.get('lifecycle')
        expected_phases = PHASES | ({'canonical_compile_unavailable'} if public else {'canonical_compile'})
        if not isinstance(lifecycle, dict) or set(lifecycle) != expected_phases:
            raise ValueError('missing lifecycle phase: ' + name)
        for phase in PHASES | (set() if public else {'canonical_compile'}):
            validate_statistics(lifecycle[phase], 11)
        if public and lifecycle['canonical_compile_unavailable'] != 'public API has no canonical SourceDocument':
            raise ValueError('invalid public API limitation')
    calibration = report.get('calibration')
    if (not isinstance(calibration, dict) or calibration.get('injected_pause_ns') != 200_000
            or calibration.get('injection_interval') != 100
            or calibration.get('injected_indices') != list(range(0, 4096, 100))):
        raise ValueError('rare-pause calibration configuration differs')
    validate_statistics(calibration.get('empty_timer'), 4096)
    observed = validate_statistics(calibration.get('injected_pause'), 4096)
    measured = observed['samples_ns']
    if any(measured[index] < 200_000 for index in calibration['injected_indices']):
        raise ValueError('injected pause was not observed')
    means = [sum(measured[index:index + 64]) / 64 for index in range(0, 4096, 64)]
    batch = validate_statistics(calibration.get('calibration_batch_means'), 64)
    if batch['samples_ns'] != means or observed['p99_9_ns'] < 200_000:
        raise ValueError('rare pause is missing or batch means differ')
    return report


def tree_identity(source):
    rows = []
    for directory, dirs, files in os.walk(source, followlinks=False):
        if any((Path(directory) / name).is_symlink() for name in dirs):
            raise ValueError('source snapshot contains a directory symlink')
        dirs[:] = sorted(name for name in dirs if name not in {'.git', 'target', '__pycache__', '.pytest_cache'})
        for name in sorted(files):
            path = Path(directory) / name
            if path.is_symlink():
                raise ValueError('source snapshot contains a symlink')
            data = path.read_bytes()
            rows.append((path.relative_to(source).as_posix(), len(data), sha(data)))
    return {'files': len(rows), 'sha256': sha(canonical(rows)),
            'cargo_lock_sha256': sha((source / 'Cargo.lock').read_bytes()),
            'probe_sha256': sha((source / 'examples/bench_fragments.rs').read_bytes())}


def outside(path, source):
    path = Path(path).resolve()
    if path == source or source in path.parents:
        raise ValueError('measurement outputs must be outside the source tree')
    return path


def tool_version(command):
    result = subprocess.run(command, capture_output=True, text=True, timeout=20, check=True)
    if len(result.stdout) > 16_384 or len(result.stderr) > 16_384:
        raise ValueError('version output exceeds bound')
    return result.stdout.strip()


def hardware():
    models = []
    cpuinfo = Path('/proc/cpuinfo')
    if cpuinfo.exists():
        for line in cpuinfo.read_text().splitlines():
            if line.startswith(('model name', 'Hardware')):
                model = line.partition(':')[2].strip()
                if model not in models:
                    models.append(model)
    affinity = sorted(os.sched_getaffinity(0)) if hasattr(os, 'sched_getaffinity') else None
    value = {'system': platform.system(), 'machine': platform.machine(), 'release': platform.release(),
             'cpu_models': models, 'cpu_affinity': affinity}
    value['host_fingerprint'] = sha(canonical([platform.node(), value]))
    return value


def capture(args):
    source, binary = args.source.resolve(), args.binary.resolve()
    output = outside(outside(args.output, source), ROOT)
    if output.exists():
        raise ValueError('capture output must be a new path')
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError('benchmark binary must already exist and be executable')
    if not args.identity.strip() or not args.build_id.strip():
        raise ValueError('source and build identities must be nonempty')
    for value, low, high in ((args.samples, 1024, 16_384), (args.rounds, 5, 101), (args.batch_calls, 32, 4096)):
        if not integer(value, low, high):
            raise ValueError('capture sampling setting outside benchmark bounds')
    if not integer(args.timeout, 1, 3600):
        raise ValueError('capture timeout outside 1..3600 seconds')
    before = tree_identity(source)
    binary_before = file_digest(binary)
    features = sorted(set(filter(None, (part.strip() for part in args.features.split(',')))))
    compiler = {'rustc': tool_version(['rustc', '-vV']), 'cargo': tool_version(['cargo', '--version'])}
    host = hardware()
    flags = {key: os.environ.get(key) for key in FLAGS}
    output.parent.mkdir(parents=True, exist_ok=True)
    # Atomic reservation prevents accidental overwrite even when a prior
    # attempt stopped before publishing a capture. Partial bounded logs survive.
    logs = output.with_name(output.name + '.logs')
    logs.mkdir(mode=0o700)
    stdout_path, stderr_path = logs / 'stdout', logs / 'stderr'
    started = time.perf_counter_ns()
    command = [sys.executable, '-c', LAUNCH, str(args.samples), str(args.rounds),
               str(args.batch_calls), '1' if args.common_routes else '0', str(binary)]
    outcome = run_bounded(command, b'', source, stdout_path, stderr_path,
                          timeout_seconds=args.timeout, max_stdout_bytes=MAX_BYTES,
                          max_stderr_bytes=MAX_STDERR)
    execution = {**asdict(outcome), 'stdout': file_digest(stdout_path), 'stderr': file_digest(stderr_path)}
    write_new(logs / 'execution.json', execution)
    if outcome.terminal != 'exited' or outcome.exit_code != 0:
        raise ValueError('benchmark failed: ' + outcome.terminal + '; bounded logs retained in ' + logs.name)
    report = validate_report(read(stdout_path))
    if report['probe_sha256'] != before['probe_sha256']:
        raise ValueError('binary embeds a different benchmark source')
    if report['debug_assertions'] or args.profile != 'release':
        raise ValueError('capture requires a release probe without debug assertions')
    if report['typed_jit_feature'] != ('typed-jit' in features):
        raise ValueError('declared features do not match the binary')
    expected_settings = {'individual_samples': args.samples, 'batch_rounds': args.rounds,
                         'calls_per_batch': args.batch_calls, 'common_routes': args.common_routes}
    if any(report['settings'][key] != value for key, value in expected_settings.items()):
        raise ValueError('binary sampling or route schedule differs from the requested experiment')
    startup, startup_execution = [], []
    for index in range(11):
        out, err = logs / f'startup-{index}.stdout', logs / f'startup-{index}.stderr'
        begin = time.perf_counter_ns()
        result = run_bounded([str(binary), '--startup'], b'', source, out, err,
                             timeout_seconds=20, max_stdout_bytes=STARTUP_STDOUT,
                             max_stderr_bytes=MAX_STDERR)
        startup.append(time.perf_counter_ns() - begin)
        record = {**asdict(result), 'stdout': file_digest(out), 'stderr': file_digest(err)}
        write_new(logs / f'startup-{index}.json', record)
        startup_execution.append(record)
        if result.terminal != 'exited' or result.exit_code != 0:
            raise ValueError('startup probe failed: ' + result.terminal + '; bounded logs retained')
        if out.read_bytes() != b'fluxr-fragments-ready\n' or err.stat().st_size:
            raise ValueError('startup-only probe output differs')
    if (before != tree_identity(source) or file_digest(binary) != binary_before
            or hardware() != host or {key: os.environ.get(key) for key in FLAGS} != flags):
        raise ValueError('source, binary, host affinity or compiler flags changed during measurement')
    value = {
        'schema': 'fluxr-fragment-capture-v1', 'source_identity': args.identity,
        'build_id': args.build_id, 'source': before,
        'binary': binary_before, 'compiler': compiler,
        'configuration': {'profile': args.profile, 'features': features,
                          'environment_flags': flags},
        'hardware': host, 'duration_ns': time.perf_counter_ns() - started,
        'process_startup': statistics(startup, 'ns per bounded startup-only process spawn, collect and reap'),
        'execution': execution, 'startup_execution': startup_execution,
        'logs_directory': logs.name, 'bounded_collection': 'posix-process-groups-v1',
        'report': report, 'source_and_binary_unchanged': True,
        'replication_scope': 'one identified build and process run; independent build/layout replication is not established',
    }
    validate_capture(value)
    write_new(output, value)
    return {'status': 'consistent', 'capture': str(output), 'cases': len(report['cases']),
            'individual_samples_per_case_route': args.samples,
            'qualification': 'empirical observations only; no statistical improvement assertion'}


def validate_execution(value, stdout_limit):
    if (not isinstance(value, dict) or value.get('terminal') != 'exited'
            or value.get('exit_code') != 0 or type(value.get('exit_code')) is not int
            or value.get('process_started') is not True or value.get('detail') is not None
            or not number(value.get('seconds'))):
        raise ValueError('fragment probe did not complete under bounded collection')
    for key, limit in (('stdout', stdout_limit), ('stderr', MAX_STDERR)):
        log = value.get(key)
        if (not isinstance(log, dict) or not digest_valid(log.get('sha256'))
                or not integer(log.get('bytes'), 0, limit)):
            raise ValueError('bounded output identity is missing: ' + key)


def validate_capture(value):
    if (not isinstance(value, dict) or value.get('schema') != 'fluxr-fragment-capture-v1'
            or value.get('source_and_binary_unchanged') is not True):
        raise ValueError('comparison requires a complete immutable-source capture')
    validate_report(value.get('report'))
    for key in ('source_identity', 'build_id'):
        if not isinstance(value.get(key), str) or not value[key].strip():
            raise ValueError('missing comparison identity')
    for key in ('compiler', 'configuration', 'hardware', 'source', 'binary'):
        if not isinstance(value.get(key), dict) or not value[key]:
            raise ValueError('missing comparison metadata: ' + key)
    compiler = value['compiler']
    if (set(compiler) != {'rustc', 'cargo'}
            or not isinstance(compiler['rustc'], str) or not compiler['rustc'].startswith('rustc ')
            or not isinstance(compiler['cargo'], str) or not compiler['cargo'].startswith('cargo ')):
        raise ValueError('compiler metadata must identify actual rustc and Cargo versions')
    configuration = value['configuration']
    features = configuration.get('features')
    flags = configuration.get('environment_flags')
    if (configuration.get('profile') != 'release' or value['report']['debug_assertions']
            or not isinstance(features, list) or not all(isinstance(name, str) and name and ',' not in name for name in features)
            or features != sorted(set(features))
            or value['report']['typed_jit_feature'] != ('typed-jit' in features)
            or not isinstance(flags, dict) or set(flags) != set(FLAGS)
            or not all(item is None or isinstance(item, str) for item in flags.values())):
        raise ValueError('release profile, feature selection or compiler flags are incomplete')
    host = value['hardware']
    if (not all(isinstance(host.get(key), str) and host[key] for key in ('system', 'machine', 'release'))
            or not digest_valid(host.get('host_fingerprint'))
            or not isinstance(host.get('cpu_models'), list)
            or not all(isinstance(model, str) and model for model in host['cpu_models'])
            or not (host.get('cpu_affinity') is None or isinstance(host['cpu_affinity'], list)
                    and host['cpu_affinity'] and all(integer(cpu, 0, 1_000_000) for cpu in host['cpu_affinity'])
                    and host['cpu_affinity'] == sorted(set(host['cpu_affinity'])))):
        raise ValueError('hardware metadata is incomplete')
    system = {'Linux': 'linux', 'Windows': 'windows', 'Darwin': 'macos'}.get(host['system'])
    machine = {'AMD64': 'x86_64', 'arm64': 'aarch64'}.get(host['machine'], host['machine'])
    if system != value['report']['os'] or machine != value['report']['arch']:
        raise ValueError('reported target differs from executing host')
    for key in ('sha256', 'cargo_lock_sha256', 'probe_sha256'):
        if not digest_valid(value['source'].get(key)):
            raise ValueError('invalid source identity hash')
    if value['report']['probe_sha256'] != value['source']['probe_sha256']:
        raise ValueError('probe source binding differs')
    if not digest_valid(value['binary'].get('sha256')):
        raise ValueError('missing binary identity')
    if not integer(value['source'].get('files'), 1, 1_000_000) or not integer(value['binary'].get('bytes'), 1, 2**40):
        raise ValueError('source inventory or executable size is missing')
    validate_statistics(value.get('process_startup'), 11)
    collection = value.get('bounded_collection')
    if collection is not None:
        if collection != 'posix-process-groups-v1':
            raise ValueError('unknown bounded fragment collection')
        validate_execution(value.get('execution'), MAX_BYTES)
        executions = value.get('startup_execution')
        if not isinstance(executions, list) or len(executions) != 11:
            raise ValueError('missing bounded startup observations')
        for record in executions:
            validate_execution(record, STARTUP_STDOUT)
        if value['process_startup']['observation_unit'] != 'ns per bounded startup-only process spawn, collect and reap':
            raise ValueError('bounded startup observation unit differs')
    elif workload_catalog(value['report']) != LEGACY_CATALOG:
        raise ValueError('current fragment catalog requires bounded collection')
    return value


def compare_statistics(baseline, candidate):
    if baseline['observation_unit'] != candidate['observation_unit']:
        raise ValueError('observation units do not match')
    if baseline['sample_count'] != candidate['sample_count']:
        raise ValueError('sample counts do not match')
    left = baseline['percentiles_p0_through_p100_ns'] + [baseline['p99_9_ns']]
    right = candidate['percentiles_p0_through_p100_ns'] + [candidate['p99_9_ns']]
    rows = []
    for label, before, after in zip([f'p{i}' for i in range(101)] + ['p99.9'], left, right):
        ratio = after / before if before > 0 else None
        if ratio is not None and not math.isfinite(ratio):
            raise ValueError('nonfinite derived timing ratio')
        rows.append({'percentile': label, 'baseline_ns': before, 'candidate_ns': after,
                     'candidate_over_baseline': ratio, 'delta_ns': after - before,
                     'observation': 'regressed' if after > before else 'improved' if after < before else 'equal'})
    # Comparing all sorted observations covers every step of the empirical
    # nearest-rank quantile function, including ranks between integer points.
    rank_pairs = list(zip(sorted(baseline['samples_ns']), sorted(candidate['samples_ns'])))
    deltas = [after - before for before, after in rank_pairs]
    worst = max(range(len(deltas)), key=deltas.__getitem__)
    return {'observation_unit': baseline['observation_unit'], 'sample_count': baseline['sample_count'],
            'all_observed_percentiles_nonregressing': all(delta <= 0 for delta in deltas),
            'all_observed_percentiles_improved': all(delta < 0 for delta in deltas),
            'all_reported_points_nonregressing': all(b <= a for a, b in zip(left, right)),
            'full_empirical_distribution': {
                'compared_order_statistics': len(deltas),
                'regressed_order_statistics': sum(delta > 0 for delta in deltas),
                'improved_order_statistics': sum(delta < 0 for delta in deltas),
                'equal_order_statistics': sum(delta == 0 for delta in deltas),
                'largest_delta_ns': deltas[worst], 'largest_delta_rank_one_based': worst + 1,
                'raw_observations_retained_in_captures': True},
            'regressed_percentiles': [r['percentile'] for r in rows if r['observation'] == 'regressed'],
            'percentiles': rows,
            'qualification': 'point estimates only; correlated samples and one build cannot establish population improvement'}


def compare(baseline, candidate, route_pair=None):
    baseline, candidate = validate_capture(baseline), validate_capture(candidate)
    if workload_catalog(baseline['report']) != workload_catalog(candidate['report']):
        raise ValueError('unmatched fragment workload catalog')
    for key in ('compiler', 'configuration', 'hardware'):
        if baseline[key] != candidate[key]:
            raise ValueError('unmatched experiment ' + key)
    for key in ('probe_sha256', 'settings', 'os', 'arch', 'target_env', 'typed_jit_feature',
                'debug_assertions', 'numerical_profile', 'telemetry', 'route_order', 'row_order'):
        if baseline['report'][key] != candidate['report'][key]:
            raise ValueError('unmatched report setting: ' + key)
    old = {row['name']: row for row in baseline['report']['cases']}
    rows, warm_pass, lifecycle_pass = [], True, True
    for row in candidate['report']['cases']:
        before = old[row['name']]
        for key in ('source_sha256', 'input_limits_oracle_sha256', 'coverage', 'validation_rows'):
            if row[key] != before[key]:
                raise ValueError('unmatched source/input/limits/oracle contract: ' + row['name'])
        common = sorted(set(before['timings']) & set(row['timings']))
        pairs = [route_pair] if route_pair else [(route, route) for route in common]
        measured = []
        for left, right in pairs:
            if left not in before['timings'] or right not in row['timings']:
                raise ValueError('requested comparison route unavailable')
            for kind in ('individual_call', 'batch_mean'):
                result = compare_statistics(before['timings'][left][kind], row['timings'][right][kind])
                warm_pass = warm_pass and result['all_observed_percentiles_nonregressing']
                measured.append({'baseline_route': left, 'candidate_route': right, 'measurement': kind, **result})
        phases, excluded_phases = [], []
        for phase in sorted((set(before['lifecycle']) & set(row['lifecycle'])) - {'canonical_compile_unavailable'}):
            if (baseline['report']['public_baseline_adapter'] != candidate['report']['public_baseline_adapter']
                    and phase in {'cold_prepare', 'cached_prepare', 'first_call'}):
                excluded_phases.append({'phase': phase, 'reason': 'public API boundary differs'})
                continue
            if phase == 'canonical_compile' and before['canonical_source_sha256'] != row['canonical_source_sha256']:
                excluded_phases.append({'phase': phase, 'reason': 'canonical source spelling differs; parse input is not matched'})
                continue
            result = compare_statistics(before['lifecycle'][phase], row['lifecycle'][phase])
            lifecycle_pass = lifecycle_pass and result['all_observed_percentiles_nonregressing']
            phases.append({'phase': phase, **result})
        rows.append({'name': row['name'], 'warm': measured, 'lifecycle': phases,
                     'lifecycle_not_compared': excluded_phases,
                     'baseline_validation_routes': before['validation_route_counts'],
                     'candidate_validation_routes': row['validation_route_counts'],
                     'baseline_only_routes': sorted(set(before['timings']) - set(row['timings'])),
                     'candidate_only_routes': sorted(set(row['timings']) - set(before['timings'])),
                     'artifact_bytes_equal': before['artifact_sha256'] == row['artifact_sha256']})
    return {'schema': 'fluxr-fragment-comparison-v1',
            'baseline_identity': baseline['source_identity'], 'candidate_identity': candidate['source_identity'],
            'baseline_build_id': baseline['build_id'], 'candidate_build_id': candidate['build_id'],
            'route_pair': route_pair, 'api_boundary_changed': bool(route_pair and route_pair[0] != route_pair[1]),
            'warm_all_observed_percentiles_nonregressing': warm_pass,
            'lifecycle_all_observed_percentiles_nonregressing': lifecycle_pass,
            'dependency_lock_equal': baseline['source']['cargo_lock_sha256'] == candidate['source']['cargo_lock_sha256'],
            'process_startup': compare_statistics(baseline['process_startup'], candidate['process_startup']),
            'cases': rows,
            'qualification': 'observational only; independent builds/layouts and uncertainty remain required for population-tail or all-percentile improvement claims',
            'public_adapter_boundary': 'API-dependent cold/cached preparation and first call remain in captures but are not compared as equivalent costs'}


def nonregression_gate(comparison, scope):
    """Optional exact empirical check; no tolerance or population inference."""
    if scope not in ('warm', 'all'):
        raise ValueError('unknown empirical nonregression scope')
    failed = []
    considered = 0

    def inspect(distribution, identity):
        nonlocal considered
        considered += 1
        if not distribution['all_observed_percentiles_nonregressing']:
            failed.append({**identity, **distribution['full_empirical_distribution']})

    for case in comparison['cases']:
        for measured in case['warm']:
            inspect(measured, {
                'case': case['name'], 'phase': 'warm', 'measurement': measured['measurement'],
                'baseline_route': measured['baseline_route'], 'candidate_route': measured['candidate_route'],
            })
        if scope == 'all':
            for measured in case['lifecycle']:
                inspect(measured, {'case': case['name'], 'phase': measured['phase']})
    if scope == 'all':
        inspect(comparison['process_startup'], {'phase': 'process_startup'})
    return {
        'scope': scope, 'passed': not failed, 'compared_distributions': considered,
        'failed_distributions': failed,
        'criterion': 'every matched empirical order statistic must be no larger in the candidate',
        'qualification': ('exact observed nonregression only; equality is allowed; no tolerance, '
                          'population guarantee or statistical significance; explicitly excluded '
                          'unmatched lifecycle phases are outside this gate'),
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    check = commands.add_parser('check', help='validate a raw report or complete capture')
    check.add_argument('report', type=Path)
    collect = commands.add_parser('capture', help='run an already-built probe under the caller resource guard')
    collect.add_argument('--binary', type=Path, required=True)
    collect.add_argument('--source', type=Path, required=True)
    collect.add_argument('--identity', required=True, help='exact input archive or commit/tree identity')
    collect.add_argument('--build-id', required=True, help='identifier for this actual compiler build')
    collect.add_argument('--profile', choices=['release'], default='release')
    collect.add_argument('--features', required=True, help='actual comma-separated Cargo features')
    collect.add_argument('--output', type=Path, required=True)
    collect.add_argument('--samples', type=int, default=10_000)
    collect.add_argument('--rounds', type=int, default=31)
    collect.add_argument('--batch-calls', type=int, default=256)
    collect.add_argument('--timeout', type=int, default=600)
    collect.add_argument('--common-routes', action='store_true', help='time the same portable/named schedule as the public API adapter')
    paired = commands.add_parser('compare', help='retain every observed percentile and regression')
    paired.add_argument('--baseline', type=Path, required=True)
    paired.add_argument('--candidate', type=Path, required=True)
    paired.add_argument('--output', type=Path, required=True)
    paired.add_argument('--baseline-route', choices=['portable', 'named', 'prepared'])
    paired.add_argument('--candidate-route', choices=['portable', 'named', 'prepared'])
    paired.add_argument('--require-nonregression', choices=['warm', 'all'],
                        help='write the complete comparison, then exit 2 if any matched empirical rank regresses; all also checks lifecycle/startup')
    args = parser.parse_args(argv)
    status = 0
    try:
        if args.command == 'check':
            value = read(args.report)
            if not isinstance(value, dict):
                raise ValueError('report root must be an object')
            report = validate_capture(value)['report'] if value.get('schema') == 'fluxr-fragment-capture-v1' else validate_report(value)
            result = {'status': 'consistent', 'cases': len(report['cases']), 'settings': report['settings'],
                      'scope': 'consistency only; not proof of execution or timing targets'}
        elif args.command == 'capture':
            if not integer(args.timeout, 1, 3600):
                raise ValueError('capture timeout outside 1..3600 seconds')
            result = capture(args)
        else:
            if bool(args.baseline_route) != bool(args.candidate_route):
                raise ValueError('both comparison routes must be supplied together')
            pair = (args.baseline_route, args.candidate_route) if args.baseline_route else None
            result = compare(read(args.baseline), read(args.candidate), pair)
            gate = None
            if args.require_nonregression:
                gate = nonregression_gate(result, args.require_nonregression)
                result['empirical_nonregression_gate'] = gate
                if not gate['passed']:
                    status = 2
            output = outside(args.output, ROOT)
            if output.exists():
                raise ValueError('comparison output must be a new path')
            output.parent.mkdir(parents=True, exist_ok=True)
            write_new(output, result)
            result = {'status': 'nonregression_failed' if status else 'consistent', 'comparison': str(output),
                      'warm_all_observed_percentiles_nonregressing': result['warm_all_observed_percentiles_nonregressing'],
                      'lifecycle_all_observed_percentiles_nonregressing': result['lifecycle_all_observed_percentiles_nonregressing'],
                      'scope': ('exact empirical nonregression gate; no population inference' if gate is not None
                                else 'all observed points retained; consistency success is not a speed gate')}
            if gate is not None:
                result['empirical_nonregression_gate'] = {
                    'scope': gate['scope'], 'passed': gate['passed'],
                    'compared_distributions': gate['compared_distributions'],
                    'failed_distributions': len(gate['failed_distributions']),
                }
        print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
        return status
    except (ValueError, TypeError, KeyError, IndexError, OverflowError, RecursionError,
            OSError, subprocess.SubprocessError) as error:
        print(json.dumps({'status': 'failed', 'error': str(error)}), file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
