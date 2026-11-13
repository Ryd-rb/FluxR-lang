#!/usr/bin/env python3
"""Pair native code with/without embedded Runtime and report dynamic route cost.

This is a local finite fixture measurement, not an eight-core challenge score.
Compilation, preparation, process startup, kernel execution and VM dispatch are
reported separately. All timing samples retain checksums and exact identities.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import struct
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fragment_performance import statistics as percentile_statistics

FIXTURES = Path(__file__).resolve().parent
SOURCE = FIXTURES.parents[1]
FLAGS = ['-C', 'target-cpu=native']
PROFILE = {'opt-level': 3, 'lto': 'fat', 'codegen-units': 1, 'panic': 'abort', 'strip': False, 'debug': False}

def sha(data): return hashlib.sha256(data).hexdigest()
def identity(path):
    data = path.read_bytes()
    return {'path': str(path), 'bytes': len(data), 'sha256': sha(data)}

def summarize(key, values):
    kind = key.split('/', 1)[1]
    batch = kind not in {'preparation', 'process_startup_to_exit'}
    if kind == 'native_kernel':
        observation = 'mean ns per native kernel invocation over a batch; loop/checksum overhead included'
    elif kind == 'preparation':
        observation = 'ns per register/install/snapshot/preparation operation; global runtime acquisition excluded'
    elif kind == 'process_startup_to_exit':
        observation = 'ns per startup-only process spawn, output collection and exit'
    else:
        observation = 'mean ns per runtime invocation over a batch; revision/type/route checks and checksum included'
    # Retain the original summary fields. statistics.median averages the two
    # middle observations for even counts; the complete quantiles use the shared
    # nearest-rank convention, so their p50 need not equal that legacy median.
    return {'samples': len(values), 'median_ns': statistics.median(values),
            'minimum_ns': min(values), 'maximum_ns': max(values),
            'unit': 'ns per invocation' if batch else 'ns per preparation/process',
            **percentile_statistics(values, observation)}

def symbol_code(binary, name):
    """Extract a named nonempty ELF64 symbol without disassembly normalization."""
    data = binary.read_bytes()
    if data[:6] != b'\x7fELF\x02\x01': raise ValueError('fixture requires little-endian ELF64')
    offset = struct.unpack_from('<Q', data, 40)[0]
    entry_size, count = struct.unpack_from('<HH', data, 58)
    sections = [struct.unpack_from('<IIQQQQIIQQ', data, offset + index * entry_size) for index in range(count)]
    matches = []
    for section in sections:
        if section[1] != 2: continue
        strings = sections[section[6]]
        names = data[strings[4]:strings[4] + strings[5]]
        for where in range(section[4], section[4] + section[5], section[9]):
            key, _, _, target, address, size = struct.unpack_from('<IBBHQQ', data, where)
            if names[key:names.find(b'\0', key)].decode() != name: continue
            assert size > 0 and 0 < target < len(sections)
            body = sections[target]
            start = body[4] + address - body[3]
            matches.append(data[start:start + size])
    if len(matches) != 1: raise ValueError('expected exactly one native kernel symbol')
    return matches[0]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--target-dir', type=Path, required=True)
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--samples', type=int, default=9)
    parser.add_argument('--kernel-iterations', type=int, default=5_000_000)
    parser.add_argument('--runtime-iterations', type=int, default=100_000)
    args = parser.parse_args()
    if not 5 <= args.samples <= 31 or not 10000 <= args.runtime_iterations <= 2_000_000 or not 100000 <= args.kernel_iterations <= 100_000_000:
        parser.error('sample or iteration count outside declared bounds')
    work, output, target = args.work.resolve(), args.output.resolve(), args.target_dir.resolve()
    if work.exists() or output.exists(): parser.error('work/output must be new directories')
    work.mkdir(parents=True); output.mkdir(parents=True)
    fixture = FIXTURES / 'performance_fixture.rs'
    native_body = fixture.read_bytes()
    before = identity(fixture)
    report = {'schema': 'fluxr-unified-performance-v1', 'passed': False, 'source_before': before,
              'commands': [], 'samples': [], 'rustflags': FLAGS, 'profile': PROFILE,
              'numerical_profile': 'signed i64; 0 <= x < 997; x*x+17 <= 992033; '
                                   'maximum permitted checksum <= 99600000000; no overflow',
              'percentile_scope': f'Empirical nearest-rank percentiles from {args.samples} batch/preparation '
                                  'observations and 20 startup observations; p99.9 equals the observed maximum. '
                                  'Kernel/runtime distributions are batch means, not individual-call tails '
                                  'or population guarantees.',
              'claim_scope': 'Exact local integer fixture and runtime dispatch measurements. No universal speed equality or leaderboard victory.',
              'started_at_unix': time.time()}
    def save(): (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    def run(label, command, cwd=None, env=None):
        command = [str(v) for v in command]; start = time.perf_counter_ns()
        result = subprocess.run(command, cwd=cwd, env=env, capture_output=True, timeout=600)
        elapsed = time.perf_counter_ns() - start
        (output / (label + '.stdout.log')).write_bytes(result.stdout)
        (output / (label + '.stderr.log')).write_bytes(result.stderr)
        report['commands'].append({'name': label, 'argv': command, 'cwd': str(cwd) if cwd else None,
                                   'exit_code': result.returncode, 'wall_ns': elapsed,
                                   'stdout_sha256': sha(result.stdout), 'stderr_sha256': sha(result.stderr)})
        save()
        assert result.returncode == 0, label
        return result.stdout.decode(), elapsed
    def cargo_files(root, package, unified):
        root.mkdir()
        profile = '\n'.join(key + ' = ' + json.dumps(value) for key, value in PROFILE.items())
        (root / 'Cargo.toml').write_text(f'[package]\nname = "{package}"\nversion = "0.1.0"\nedition = "2024"\n'
            '[features]\nunified = []\ndefault = ' + ('["unified"]' if unified else '[]') + '\n[profile.release]\n' + profile + '\n')
        (root / 'Cargo.lock').write_text(f'version = 4\n\n[[package]]\nname = "{package}"\nversion = "0.1.0"\n')
    def checksum(n):
        cycle = sum((x * x + 17) % 997 for x in range(997))
        return (n // 997) * cycle + sum((x * x + 17) % 997 for x in range(n % 997))
    try:
        env = os.environ.copy()
        for key in list(env):
            if key == 'RUSTFLAGS' or key.startswith('FLUXR_CONTROL_'): env.pop(key)
        env['CARGO_ENCODED_RUSTFLAGS'] = '\x1f'.join(FLAGS)
        run('rustc-version', ['rustc', '-vV'], env=env)
        run('cargo-version', ['cargo', '--version'], env=env)
        direct = work / 'direct-source'; cargo_files(direct, 'fluxr-control-native-kernel', False)
        (direct / 'src').mkdir(); (direct / 'src/main.rs').write_bytes(native_body)
        run('build-direct', ['cargo', 'build', '--release', '--locked', '--offline', '--jobs', '1', '--target-dir', target], direct, env)
        binaries = {}
        binaries['direct'] = work / 'direct-kernel'
        binaries['direct'].write_bytes((target / 'release/fluxr-control-native-kernel').read_bytes()); binaries['direct'].chmod(0o755)
        unified = work / 'unified-source'; cargo_files(unified, 'fluxr-unified-kernel', True)
        checked = b'fluxr 1\npub calc 1 x int int ret rem add mul x x 17 997\n'
        authored = checked + b'rust ' + str(len(native_body)).encode() + b'\n' + native_body + b'\n'
        (unified / 'main.fluxr').write_bytes(authored)
        files = [{'source': name, 'output': name, 'mode': 'copy', 'executable': False} for name in ('Cargo.toml', 'Cargo.lock')]
        files.append({'source': 'main.fluxr', 'output': 'src/main.rs', 'mode': 'source', 'executable': False})
        manifest = unified / 'project.json'
        manifest.write_text(json.dumps({'format': 'fluxr-project-v1', 'files': files, 'rustflags': FLAGS}, indent=2) + '\n')
        emitted = work / 'unified-emitted'
        run('build-unified', [args.cli, 'build', manifest, emitted, '--target-dir', target, '--logs', output / 'unified-cargo', '--compiler', args.cli], env=env)
        binaries['unified'] = work / 'unified-kernel'
        binaries['unified'].write_bytes((target / 'release/fluxr-unified-kernel').read_bytes()); binaries['unified'].chmod(0o755)
        report['emission'] = json.loads((emitted / 'fluxr-receipt.json').read_text())
        report['binaries_before'] = {arm: identity(binary) for arm, binary in binaries.items()}
        symbols = {}
        for arm, binary in binaries.items():
            code = symbol_code(binary, 'fluxr_unified_probe_kernel')
            (output / (arm + '-kernel-machine-code.bin')).write_bytes(code)
            symbols[arm] = {'bytes': len(code), 'sha256': sha(code), 'hex': code.hex()}
        report['native_kernel_code'] = symbols
        report['native_kernel_machine_code_equal'] = symbols['direct'] == symbols['unified']
        runtime_env = {key: value for key, value in env.items() if not key.startswith(('RUST', 'CARGO'))}
        empty = work / 'empty-path'; empty.mkdir(); runtime_env['PATH'] = str(empty)
        report['runtime_path_empty'] = True
        for index in range(args.samples):
            order = ['direct', 'unified'] if index % 2 == 0 else ['unified', 'direct']
            for arm in order:
                stdout, _ = run(f'kernel-{index}-{arm}', [binaries[arm], 'kernel', str(args.kernel_iterations)], env=runtime_env)
                sample = json.loads(stdout)
                assert sample['iterations'] == args.kernel_iterations and sample['checksum'] == checksum(args.kernel_iterations), sample
                report['samples'].append({'sample': index, 'arm': arm, **sample})
            stdout, _ = run('runtime-' + str(index), [binaries['unified'], 'runtime', str(args.runtime_iterations)], env=runtime_env)
            observed = [json.loads(line) for line in stdout.splitlines()]
            kinds = {row['kind'] for row in observed}
            expected = {'preparation', 'prepared_aot', 'prepared_vm', 'prepared_host', 'bundled_aot_snapshot', 'injected_vm_snapshot', 'injected_vm_registry', 'injected_host_snapshot'}
            assert len(observed) == 8 and kinds == expected, observed
            for sample in observed:
                if sample['kind'] != 'preparation':
                    assert sample['iterations'] == args.runtime_iterations and sample['checksum'] == checksum(args.runtime_iterations), sample
                    assert sample['route'] == ('aot' if sample['kind'] in {'prepared_aot', 'bundled_aot_snapshot'} else 'vm'), sample
                report['samples'].append({'sample': index, 'arm': 'unified', **sample})
            save()
        for index in range(20):
            for arm in (['direct', 'unified'] if index % 2 == 0 else ['unified', 'direct']):
                stdout, elapsed = run(f'startup-{index}-{arm}', [binaries[arm], 'startup'], env=runtime_env)
                assert stdout == 'ready\n'
                report['samples'].append({'sample': index, 'arm': arm, 'kind': 'process_startup_to_exit', 'elapsed_ns': elapsed})
        groups = {}
        for sample in report['samples']:
            key = sample['arm'] + '/' + sample['kind']
            groups.setdefault(key, []).append(sample['elapsed_ns'] / sample.get('iterations', 1))
        report['measurements'] = {key: summarize(key, values) for key, values in groups.items()}
        report['native_kernel_median_ratio_unified_over_direct'] = statistics.median(groups['unified/native_kernel']) / statistics.median(groups['direct/native_kernel'])
        report['binaries_after'] = {arm: identity(binary) for arm, binary in binaries.items()}
        report['binaries_unchanged'] = report['binaries_before'] == report['binaries_after']
        report['source_after'] = identity(fixture)
        report['source_unchanged'] = report['source_before'] == report['source_after']
        report['passed'] = (report['source_unchanged'] and report['binaries_unchanged']
                            and report['native_kernel_machine_code_equal'] and report['emission']['runtime_required'] is True)
    except BaseException as error:
        report['error'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        report['finished_at_unix'] = time.time(); save()
    return 0 if report['passed'] else 1

if __name__ == '__main__':
    raise SystemExit(main())
