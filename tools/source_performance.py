#!/usr/bin/env python3
"""Capture and compare bounded source API latency distributions.

Run the already-built, trusted probe under the resource coordinator. Capture
uses the existing POSIX process-group collector; check/compare are offline.
Consistency does not authenticate execution or establish a performance pass.
Only typed-jit is observed from compiled cfg; other features remain declared.
"""
from __future__ import annotations

import argparse
from dataclasses import asdict
import json
import os
from pathlib import Path
import subprocess
import sys
import time

import fragment_performance as fp
from eval.runner import file_digest, run_bounded

ROOT = Path(__file__).resolve().parents[1]
LEGACY_CASES = ('checked_small', 'checked_updates_64', 'native_unquoted_64',
                'native_quoted_64', 'native_unicode_64', 'native_raw_64')
CASES = LEGACY_CASES + ('checked_floats_raw_64', 'checked_floats_special_64')
DECIMAL_CASES = CASES + ('checked_floats_decimal_64',)
PHASES = ('parse_document', 'compile_checked_projection', 'print_checked_projection')
UNIT = 'ns per individual source API call; disposal and validation excluded'
TIMER_UNIT = 'ns per empty timer pair'
FEATURE_SCOPE = 'typed-jit observed from compiled cfg; other feature selections are declared, not inferred'
VALIDATION = ('exact source AST, projection text, native spans/digest, artifact and printed round trip; '
              'independent run(1)=18')
FLAGS = fp.FLAGS + ('CARGO_BUILD_TARGET',)
MAX_STDOUT = 8 * 1024 * 1024
MAX_STDERR = 64 * 1024
# execv preserves the collector's process group and sets only this probe's knobs.
# Python startup is outside every timing interval reported by the Rust probe.
LAUNCH = ("import os,sys; os.environ['FLUXR_SOURCE_SAMPLES']=sys.argv[1]; "
          "os.environ['FLUXR_SOURCE_DECIMAL']=sys.argv[2]; os.execv(sys.argv[3],[sys.argv[3]])")


def tree_identity(source):
    value = fp.tree_identity(source)
    value['probe_sha256'] = fp.sha((source / 'examples/bench_source.rs').read_bytes())
    return value


def validate_timing(value, count, unit=UNIT):
    fp.validate_statistics(value, count)
    if (value['observation_unit'] != unit
            or not all(fp.integer(sample, 0, 2**64 - 1) for sample in value['samples_ns'])):
        raise ValueError('source timings require integer nanoseconds and the exact API observation unit')


def validate_report(report):
    if (not isinstance(report, dict) or report.get('schema') != 'fluxr-source-performance-v1'
            or not fp.digest_valid(report.get('probe_sha256'))
            or report.get('debug_assertions') is not False
            or type(report.get('typed_jit_feature')) is not bool
            or not all(isinstance(report.get(k), str) and report[k] for k in ('os', 'arch', 'target_env'))):
        raise ValueError('source probe identity, release build or target metadata differs')
    settings = report.get('settings')
    if (not isinstance(settings, dict) or not fp.integer(settings.get('samples'), 1024, 16_384)
            or settings.get('warmup') != 32 or type(settings.get('warmup')) is not int
            or settings.get('case_order') != 'fixed; phases parse, compile, print'
            or type(settings.get('decimal_literals', False)) is not bool):
        raise ValueError('source experiment sampling or ordering differs')
    rows = report.get('cases')
    expected = (DECIMAL_CASES,) if settings.get('decimal_literals', False) else (LEGACY_CASES, CASES)
    if (not isinstance(rows, list) or any(not isinstance(row, dict) for row in rows)
            or tuple(row.get('name') for row in rows) not in expected):
        raise ValueError('missing, duplicate, reordered or unknown source workload')
    for row in rows:
        name = row['name']
        for key in ('source_sha256', 'artifact_sha256', 'canonical_sha256'):
            if not fp.digest_valid(row.get(key)):
                raise ValueError('missing source/artifact/canonical binding: ' + name)
        if (not fp.integer(row.get('source_bytes'), 1, 8 * 1024 * 1024)
                or not fp.integer(row.get('canonical_bytes'), 1, 8 * 1024 * 1024)
                or type(row.get('native_items')) is not int
                or row['native_items'] != (64 if name.startswith('native_') else 0)
                or type(row.get('checked_functions')) is not int
                or row['checked_functions'] != (65 if name.startswith('checked_') and name.endswith('_64') else 1)
                or row.get('validation') != VALIDATION):
            raise ValueError('source workload shape or semantic validation differs: ' + name)
        phases = row.get('phases')
        if not isinstance(phases, dict) or set(phases) != set(PHASES):
            raise ValueError('missing or unknown source lifecycle phase: ' + name)
        for phase in PHASES:
            validate_timing(phases[phase], settings['samples'])
    validate_timing(report.get('timer_calibration'), 4096, TIMER_UNIT)
    return report


def validate_capture(value):
    if (not isinstance(value, dict) or value.get('schema') != 'fluxr-source-capture-v1'
            or value.get('source_and_binary_unchanged') is not True):
        raise ValueError('comparison requires a complete immutable-source capture')
    report = validate_report(value.get('report'))
    for key in ('source_identity', 'build_id'):
        if not isinstance(value.get(key), str) or not value[key].strip():
            raise ValueError('missing source/build identity')
    for key in ('source', 'binary', 'compiler', 'configuration', 'hardware', 'execution'):
        if not isinstance(value.get(key), dict):
            raise ValueError('missing capture metadata: ' + key)
    source, binary, compiler = value['source'], value['binary'], value['compiler']
    if (any(not fp.digest_valid(source.get(key)) for key in ('sha256', 'cargo_lock_sha256', 'probe_sha256'))
            or source['probe_sha256'] != report['probe_sha256']
            or not fp.integer(source.get('files'), 1, 1_000_000)
            or not fp.digest_valid(binary.get('sha256'))
            or not fp.integer(binary.get('bytes'), 1, 2**40)):
        raise ValueError('source/probe/lock/binary identity is incomplete')
    if (set(compiler) != {'rustc', 'cargo'}
            or not all(isinstance(compiler[k], str) and compiler[k].startswith(k + ' ') for k in compiler)):
        raise ValueError('actual rustc and Cargo versions are required')
    config = value['configuration']
    features, flags = config.get('features'), config.get('environment_flags')
    if (config.get('profile') != 'release' or config.get('feature_observation') != FEATURE_SCOPE
            or not isinstance(features, list)
            or not all(isinstance(name, str) and name and ',' not in name for name in features)
            or features != sorted(set(features))
            or report['typed_jit_feature'] != ('typed-jit' in features)
            or config.get('samples') != report['settings']['samples']
            or type(config.get('samples')) is not int
            or not isinstance(flags, dict) or set(flags) != set(FLAGS)
            or not all(item is None or isinstance(item, str) for item in flags.values())):
        raise ValueError('release, observed feature, declared flags or sampling configuration differs')
    host = value['hardware']
    affinity = host.get('cpu_affinity')
    if (not all(isinstance(host.get(k), str) and host[k] for k in ('system', 'machine', 'release'))
            or not fp.digest_valid(host.get('host_fingerprint'))
            or not isinstance(host.get('cpu_models'), list)
            or not all(isinstance(model, str) and model for model in host['cpu_models'])
            or not (affinity is None or isinstance(affinity, list) and affinity
                    and all(fp.integer(cpu, 0, 1_000_000) for cpu in affinity)
                    and affinity == sorted(set(affinity)))):
        raise ValueError('hardware metadata is incomplete')
    system = {'Linux': 'linux', 'Windows': 'windows', 'Darwin': 'macos'}.get(host['system'])
    machine = {'AMD64': 'x86_64', 'arm64': 'aarch64'}.get(host['machine'], host['machine'])
    if (system, machine) != (report['os'], report['arch']):
        raise ValueError('reported target differs from executing host')
    execution = value['execution']
    if (execution.get('terminal') != 'exited' or execution.get('exit_code') != 0
            or type(execution.get('exit_code')) is not int
            or execution.get('process_started') is not True
            or execution.get('detail') is not None
            or not fp.number(execution.get('seconds'))):
        raise ValueError('probe did not complete under bounded collection')
    for key, limit in (('stdout', MAX_STDOUT), ('stderr', MAX_STDERR)):
        log = execution.get(key)
        if (not isinstance(log, dict) or not fp.digest_valid(log.get('sha256'))
                or not fp.integer(log.get('bytes'), 0, limit)):
            raise ValueError('bounded output identity is missing: ' + key)
    return value


def capture(args):
    source, binary = args.source.resolve(), args.binary.resolve()
    output = fp.outside(fp.outside(args.output, source), ROOT)
    if output.exists() or not args.identity.strip() or not args.build_id.strip():
        raise ValueError('capture needs a new external output and exact source/build identities')
    if (not fp.integer(args.samples, 1024, 16_384) or not fp.integer(args.timeout, 1, 3600)
            or args.profile != 'release'):
        raise ValueError('capture configuration outside probe bounds')
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError('probe binary must already exist and be executable')
    before, binary_before = tree_identity(source), file_digest(binary)
    compiler = {'rustc': fp.tool_version(['rustc', '-vV']), 'cargo': fp.tool_version(['cargo', '--version'])}
    host = fp.hardware()
    flags = {key: os.environ.get(key) for key in FLAGS}
    features = sorted(set(filter(None, (part.strip() for part in args.features.split(',')))))
    output.parent.mkdir(parents=True, exist_ok=True)
    # Atomically reserve a private log directory; the collector cannot overwrite
    # another run's files. Failed/limited attempts retain their bounded output.
    logs = output.with_name(output.name + '.logs')
    logs.mkdir(mode=0o700)
    stdout, stderr = logs / 'stdout', logs / 'stderr'
    started = time.perf_counter_ns()
    decimal_literals = bool(getattr(args, 'decimal_literals', False))
    outcome = run_bounded([sys.executable, '-c', LAUNCH, str(args.samples), str(int(decimal_literals)), str(binary)], b'', source,
                          stdout, stderr, timeout_seconds=args.timeout,
                          max_stdout_bytes=MAX_STDOUT, max_stderr_bytes=MAX_STDERR)
    execution = {**asdict(outcome), 'stdout': file_digest(stdout), 'stderr': file_digest(stderr)}
    fp.write_new(logs / 'execution.json', execution)
    if outcome.terminal != 'exited' or outcome.exit_code != 0:
        raise ValueError('source probe failed: ' + outcome.terminal + '; bounded logs retained in ' + logs.name)
    report = validate_report(fp.read(stdout))
    if (report['probe_sha256'] != before['probe_sha256']
            or report['settings']['samples'] != args.samples
            or report['settings'].get('decimal_literals', False) != decimal_literals
            or report['typed_jit_feature'] != ('typed-jit' in features)):
        raise ValueError('binary probe, actual typed-jit feature or requested sampling differs')
    if (tree_identity(source) != before or file_digest(binary) != binary_before
            or fp.hardware() != host or {key: os.environ.get(key) for key in FLAGS} != flags):
        raise ValueError('source, binary, host affinity or compiler flags changed during measurement')
    value = {'schema': 'fluxr-source-capture-v1', 'source_identity': args.identity,
             'build_id': args.build_id, 'source': before, 'binary': binary_before,
             'compiler': compiler, 'hardware': host,
             'configuration': {'profile': args.profile, 'features': features, 'environment_flags': flags,
                               'feature_observation': FEATURE_SCOPE, 'samples': args.samples},
             'execution': execution, 'logs_directory': logs.name,
             'duration_ns': time.perf_counter_ns() - started,
             'source_and_binary_unchanged': True, 'report': report,
             'replication_scope': 'one identified build and process; compiler build commands remain in the identified external build receipt'}
    validate_capture(value)
    fp.write_new(output, value)
    return {'status': 'consistent', 'capture': str(output), 'cases': len(report['cases']),
            'qualification': 'empirical observations only; no performance pass or authenticated-build claim'}


def compare(baseline, candidate):
    baseline, candidate = validate_capture(baseline), validate_capture(candidate)
    for key in ('compiler', 'configuration', 'hardware'):
        if baseline[key] != candidate[key]:
            raise ValueError('unmatched experiment ' + key)
    if baseline['source']['cargo_lock_sha256'] != candidate['source']['cargo_lock_sha256']:
        raise ValueError('dependency locks differ')
    for key in ('probe_sha256', 'settings', 'os', 'arch', 'target_env', 'typed_jit_feature', 'debug_assertions'):
        if baseline['report'][key] != candidate['report'][key]:
            raise ValueError('unmatched report setting: ' + key)
    if ([row['name'] for row in baseline['report']['cases']]
            != [row['name'] for row in candidate['report']['cases']]):
        raise ValueError('unmatched source workload catalog')
    rows, nonregressing = [], True
    for before, after in zip(baseline['report']['cases'], candidate['report']['cases']):
        for key in ('name', 'source_bytes', 'source_sha256', 'artifact_sha256',
                    'native_items', 'checked_functions', 'validation'):
            if before[key] != after[key]:
                raise ValueError('unmatched source input or checked semantic artifact: ' + key)
        phases = []
        for phase in PHASES:
            result = fp.compare_statistics(before['phases'][phase], after['phases'][phase])
            nonregressing = nonregressing and result['all_observed_percentiles_nonregressing']
            phases.append({'phase': phase, **result})
        rows.append({'name': after['name'], 'phases': phases, 'canonical_output': {
            'changed': before['canonical_sha256'] != after['canonical_sha256'],
            'baseline_sha256': before['canonical_sha256'], 'candidate_sha256': after['canonical_sha256'],
            'baseline_bytes': before['canonical_bytes'], 'candidate_bytes': after['canonical_bytes'],
            'byte_delta': after['canonical_bytes'] - before['canonical_bytes'],
            'comparison_basis': 'same checked semantic artifact; each current printer verifies its own canonical round trip'}})
    return {'schema': 'fluxr-source-comparison-v1',
            'baseline_identity': baseline['source_identity'], 'candidate_identity': candidate['source_identity'],
            'baseline_build_id': baseline['build_id'], 'candidate_build_id': candidate['build_id'],
            'all_observed_percentiles_nonregressing': nonregressing, 'cases': rows,
            'timer_calibration': fp.compare_statistics(baseline['report']['timer_calibration'], candidate['report']['timer_calibration']),
            'qualification': 'observational only; all ranks retained; independent builds/processes and uncertainty remain required for population-tail claims'}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    check = commands.add_parser('check')
    check.add_argument('report', type=Path)
    collect = commands.add_parser('capture')
    for key in ('binary', 'source', 'output'):
        collect.add_argument('--' + key, type=Path, required=True)
    for key in ('identity', 'build-id', 'features'):
        collect.add_argument('--' + key, required=True)
    collect.add_argument('--profile', choices=['release'], default='release')
    collect.add_argument('--samples', type=int, default=10_000)
    collect.add_argument('--decimal-literals', action='store_true',
                         help='include the current-grammar decimal float workload; use the same setting in both arms')
    collect.add_argument('--timeout', type=int, default=600)
    paired = commands.add_parser('compare')
    for key in ('baseline', 'candidate', 'output'):
        paired.add_argument('--' + key, type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == 'check':
            value = fp.read(args.report)
            if not isinstance(value, dict):
                raise ValueError('report root must be an object')
            report = (validate_capture(value)['report'] if value.get('schema') == 'fluxr-source-capture-v1'
                      else validate_report(value))
            result = {'status': 'consistent', 'cases': len(report['cases']), 'settings': report['settings'],
                      'scope': 'consistency only; not proof of execution or performance targets'}
        elif args.command == 'capture':
            result = capture(args)
        else:
            result = compare(fp.read(args.baseline), fp.read(args.candidate))
            output = fp.outside(args.output, ROOT)
            output.parent.mkdir(parents=True, exist_ok=True)
            fp.write_new(output, result)
            result = {'status': 'consistent', 'comparison': str(output),
                      'all_observed_percentiles_nonregressing': result['all_observed_percentiles_nonregressing'],
                      'scope': 'all observed ranks retained; consistency success is not a speed gate'}
        print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
        return 0
    except (ValueError, TypeError, KeyError, IndexError, OverflowError, RecursionError,
            OSError, subprocess.SubprocessError) as error:
        print(json.dumps({'status': 'failed', 'error': str(error)}), file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
