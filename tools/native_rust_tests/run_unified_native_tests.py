#!/usr/bin/env python3
"""Run native interoperability oracles through mandatory Runtime-bound projects.

Cargo target selection is test tooling. Emission and Runtime bundling use only
the public canonical builder; no test source is emitted for standalone rustc.
"""
from __future__ import annotations
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parent

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def main(args):
    output = args.output.resolve()
    target = args.target_dir.resolve()
    if target.is_relative_to(output) or output.is_relative_to(target):
        raise ValueError('Cargo target and retained/project output must be separate')
    output.mkdir(parents=True, exist_ok=False)
    cargo = shutil.which('cargo')
    if cargo is None:
        raise RuntimeError('cargo unavailable; no qualification was executed')
    frontend = args.frontend.resolve()
    paths = sorted(p for p in ROOT.rglob('*') if p.is_file() and '__pycache__' not in p.parts)
    before = {p.relative_to(ROOT).as_posix(): digest(p) for p in paths}
    frontend_before = digest(frontend)
    spec = importlib.util.spec_from_file_location('fluxr_unified_test_builder', frontend)
    builder = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = builder
    spec.loader.exec_module(builder)
    rows = []
    report = {'schema': 'fluxr-unified-native-capabilities-oracle-v1', 'passed': False,
              'commands': rows, 'sources_before': before, 'frontend_sha256_before': frontend_before,
              'runtime_required': True, 'language_version': 1, 'postgres_requested': args.postgres,
              'scope': 'Finite paired Rust/native-interop fixtures with embedded Runtime admission. No universal API or speed claim.',
              'planned': {'capabilities': 30, 'semantics': 5, 'postgres': 5,
                          'compiler_rejections': 9, 'additional_runtime_tests_per_harness': 1}}

    def save():
        (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')

    def run(name, command, *, cwd=None, expect=0, diagnostic=None, fatal=True):
        command = [str(v) for v in command]
        start = time.monotonic()
        env = os.environ.copy()
        for key in list(env):
            if key == 'RUSTFLAGS' or key.startswith('FLUXR_CONTROL_'):
                env.pop(key)
        env['CARGO_ENCODED_RUSTFLAGS'] = ''
        env['CARGO_INCREMENTAL'] = '0'
        completed = subprocess.run(command, cwd=cwd, env=env, capture_output=True, timeout=600)
        (output / (name + '.stdout.log')).write_bytes(completed.stdout)
        (output / (name + '.stderr.log')).write_bytes(completed.stderr)
        records = []
        for line in (completed.stdout + b'\n' + completed.stderr).splitlines():
            try:
                value = json.loads(line)
                if isinstance(value, dict): records.append(value)
            except ValueError:
                pass
        passed = completed.returncode == 0 if expect == 0 else completed.returncode != 0
        codes = []
        for record in records:
            message = record.get('message', record)
            if isinstance(message, dict) and isinstance(message.get('code'), dict):
                codes.append(message['code'].get('code'))
        if diagnostic is not None:
            passed = passed and diagnostic in codes
        row = {'name': name, 'command': command, 'cwd': str(cwd) if cwd else None,
               'exit': completed.returncode, 'wall_seconds': time.monotonic() - start,
               'passed': passed, 'stdout_sha256': hashlib.sha256(completed.stdout).hexdigest(),
               'stderr_sha256': hashlib.sha256(completed.stderr).hexdigest()}
        if diagnostic is not None:
            row.update(expected_diagnostic=diagnostic, observed_diagnostics=codes)
        summary = re.search(rb'test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;', completed.stdout)
        if summary:
            row['rust_test_counts'] = dict(zip(['passed', 'failed', 'ignored', 'measured', 'filtered_out'], map(int, summary.groups())))
            row['embedded_runtime_test_passed'] = b'test __fluxr_unified_runtime_is_embedded ... ok' in completed.stdout
        if b'unix_filesystem_listener' in completed.stdout and b'PermissionDenied' in completed.stdout and b'Operation not permitted' in completed.stdout:
            row['environment_observation'] = 'filesystem UnixListener::bind denied with EPERM; required full gate remains failed'
        rows.append(row)
        (output / 'commands.json').write_text(json.dumps(rows, indent=2) + '\n')
        save()
        if not passed and fatal:
            raise RuntimeError(name + ' failed; inspect retained logs')
        return row, records

    def project(label, sources, kind):
        source_root = output / (label + '-source')
        source_root.mkdir()
        package = 'fluxr-unified-' + label + '-oracles'
        cargo_toml = ('[package]\nname = "' + package + '"\nversion = "0.1.0"\nedition = "2024"\n'
                      'autobins = false\nautolib = false\nautotests = false\n'
                      '[profile.release]\nopt-level = 3\ncodegen-units = 1\npanic = "unwind"\n')
        entries = []
        for name, source in sources:
            destination = ('tests/' if kind == 'test' else 'src/bin/') + name + '.rs'
            authored = name + '.fluxr'
            (source_root / authored).write_bytes(source.read_bytes())
            cargo_toml += f'\n[[{kind}]]\nname = "{name}"\npath = "{destination}"\n'
            entries.append({'source': authored, 'output': destination, 'mode': 'source', 'executable': False})
        (source_root / 'Cargo.toml').write_text(cargo_toml)
        (source_root / 'Cargo.lock').write_text(f'version = 4\n\n[[package]]\nname = "{package}"\nversion = "0.1.0"\n')
        for name in ('Cargo.toml', 'Cargo.lock'):
            entries.append({'source': name, 'output': name, 'mode': 'copy', 'executable': False})
        manifest = source_root / 'project.json'
        manifest.write_text(json.dumps({'format': 'fluxr-project-v1', 'rustflags': [], 'files': entries}, indent=2) + '\n')
        emitted = output / (label + '-emitted')
        run('emit-' + label, [sys.executable, '-B', frontend, 'export', manifest, emitted])
        receipt = json.loads((emitted / 'fluxr-receipt.json').read_text())
        if receipt.get('runtime_required') is not True or receipt.get('language_version') != 1:
            raise RuntimeError('canonical emitter did not bind the mandatory Runtime')
        pinned = builder.load_project(manifest)
        if pinned.receipt != receipt:
            raise RuntimeError('builder source changed during export/projection preparation')
        resolution_env = os.environ.copy()
        for key in list(resolution_env):
            if key == 'RUSTFLAGS' or key.startswith('FLUXR_CONTROL_'): resolution_env.pop(key)
        resolution_env['CARGO_ENCODED_RUSTFLAGS'] = ''
        builder.resolve_pinned_lock(pinned, emitted, output / (label + '-resolution'),
                                    SimpleNamespace(cargo=cargo, online=False, timeout=600), resolution_env)
        report[label + '_emission'] = pinned.receipt
        return emitted

    def run_harness(project_dir, target_name, profile, label, original_count, threads):
        command = [cargo, 'test', '--locked', '--offline', '--jobs', '1', '--target-dir', target,
                   '--test', target_name, '--no-run', '--message-format=json']
        if profile == 'release': command.append('--release')
        _, messages = run('compile-' + label, command, cwd=project_dir)
        executable = [Path(row['executable']) for row in messages
                      if row.get('reason') == 'compiler-artifact' and row.get('executable')
                      and row.get('target', {}).get('name') == target_name and row.get('profile', {}).get('test')]
        if len(executable) != 1 or not executable[0].is_file():
            raise RuntimeError('Cargo did not identify exactly one test executable')
        binary_before = digest(executable[0])
        row, _ = run(label, [executable[0], '--test-threads=' + str(threads)], fatal=False)
        counts = row.get('rust_test_counts', {})
        complete = (counts.get('passed', -999) + counts.get('failed', -999) == original_count + 1
                    and all(counts.get(key) == 0 for key in ('ignored', 'measured', 'filtered_out'))
                    and row.get('embedded_runtime_test_passed') is True)
        row.update(binary=str(executable[0]), binary_sha256=binary_before,
                   binary_unchanged=digest(executable[0]) == binary_before,
                   complete_original_and_runtime_cases=complete)
        row['passed'] = row['passed'] and complete and row['binary_unchanged']
        save()

    try:
        run('rustc-version', [shutil.which('rustc') or 'rustc', '-vV'])
        run('cargo-version', [cargo, '--version'])
        positive_sources = [(name, ROOT / (name + '.fxn')) for name in ('capabilities', 'structured_semantics', 'postgres_wire')]
        positive = project('positive', positive_sources, 'test')
        # The fixture has two Unix-only socket cases.
        capability_count = 30 if os.name == 'posix' else 28
        for profile in ('debug', 'release'):
            run_harness(positive, 'capabilities', profile, 'capabilities-' + profile, capability_count, 4)
            run_harness(positive, 'structured_semantics', profile, 'semantics-' + profile, 5, 2)
        cases = json.loads((ROOT / 'compile_fail/manifest.json').read_text())['cases']
        negative_sources = [(Path(case['file']).stem, ROOT / 'compile_fail' / Path(case['file']).with_suffix('.fxn')) for case in cases]
        negative = project('negative', negative_sources, 'bin')
        for case in cases:
            name = Path(case['file']).stem
            run('reject-' + name, [cargo, 'build', '--locked', '--offline', '--jobs', '1', '--target-dir', target,
                                  '--bin', name, '--message-format=json'], cwd=negative,
                expect=1, diagnostic=case['diagnostic'], fatal=False)
        if args.postgres:
            run_harness(positive, 'postgres_wire', 'release', 'postgres-live', 5, 2)
        report['groups'] = {
            'structured_semantics': len([r for r in rows if r['name'].startswith('semantics-')]) == 2 and all(r['passed'] for r in rows if r['name'].startswith('semantics-')),
            'compile_rejections': len([r for r in rows if r['name'].startswith('reject-')]) == 9 and all(r['passed'] for r in rows if r['name'].startswith('reject-')),
            'postgres_live': next((r['passed'] for r in rows if r['name'] == 'postgres-live'), None),
            'capabilities_full': len([r for r in rows if r['name'].startswith('capabilities-')]) == 2 and all(r['passed'] for r in rows if r['name'].startswith('capabilities-')),
        }
        report['passed'] = all(r['passed'] for r in rows) and (not args.postgres or report['groups']['postgres_live'] is True)
    except BaseException as error:
        report['error'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        report['sources_after'] = {p.relative_to(ROOT).as_posix(): digest(p) for p in paths}
        report['sources_unchanged'] = before == report['sources_after']
        report['frontend_sha256_after'] = digest(frontend)
        report['frontend_unchanged'] = frontend_before == report['frontend_sha256_after']
        report['passed'] = report['passed'] and report['sources_unchanged'] and report['frontend_unchanged']
        save()
    print(json.dumps({'passed': report['passed'], 'groups': report['groups'], 'output': str(output)}))
    return 0 if report['passed'] else 1
