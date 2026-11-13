#!/usr/bin/env python3
"""Independent executable/runtime/host integration qualification.

Run under the shared resource coordinator. Builds real standalone applications,
removes compiler tools from their runtime PATH, injects checked source over the
authenticated loopback endpoint, and drives real native threads/network/PG.
"""
from __future__ import annotations
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import subprocess
import sys
import time

FIXTURES = Path(__file__).resolve().parent
SOURCE = FIXTURES.parents[1]
MAX_WIRE = 2 * 1024 * 1024

def sha(data):
    return hashlib.sha256(data).hexdigest()

def identity(path):
    data = path.read_bytes()
    return {'path': str(path), 'bytes': len(data), 'sha256': sha(data)}

def free_port():
    with socket.socket() as stream:
        stream.bind(('127.0.0.1', 0))
        return stream.getsockname()[1]

def source_bytes(native, declarations='pub embedded_marker 0 int ret 314159\n'):
    return ('fluxr 1\n' + declarations).encode() + b'rust ' + str(len(native)).encode() + b'\n' + native + b'\n'

def json_bytes(value):
    return json.dumps(value, separators=(',', ':'), allow_nan=False).encode() + b'\n'

def exchange(address, payload, *, timeout=10):
    host, port = address.rsplit(':', 1)
    with socket.create_connection((host, int(port)), timeout=timeout) as stream:
        stream.settimeout(timeout)
        stream.sendall(payload)
        stream.shutdown(socket.SHUT_WR)
        data = bytearray()
        while not data.endswith(b'\n'):
            part = stream.recv(min(8192, MAX_WIRE + 1 - len(data)))
            if not part:
                raise RuntimeError('missing or truncated control response')
            data.extend(part)
            if len(data) > MAX_WIRE:
                raise RuntimeError('oversized control response')
        result = json.loads(data)
        if result.get('version') != 1 or type(result.get('ok')) is not bool:
            raise RuntimeError('invalid control envelope')
        return result

class Control:
    def __init__(self, address, token):
        self.address, self.token = address, token
    def request(self, command, *, token=None, version=1):
        return exchange(self.address, json_bytes({'version': version, 'token': self.token if token is None else token, 'command': command}))
    def okay(self, command):
        response = self.request(command)
        assert response['ok'] is True, response
        assert response.get('error') is None, response
        return response['result']
    def rejected(self, command, code=None):
        response = self.request(command)
        assert response['ok'] is False and isinstance(response.get('error'), dict), response
        if code is not None: assert response['error']['code'] == code, response
        return response['error']
    def install(self, name, source, revision=None):
        return self.okay({'op': 'install', 'name': name, 'expected_revision': revision, 'source': source})
    def call(self, name, export, arguments=None, inputs=None, limits=None):
        command = {'op': 'call', 'name': name, 'export': export, 'arguments': arguments or [], 'inputs': inputs or []}
        if limits is not None: command['limits'] = limits
        return self.okay(command)
    def value(self, name, export, arguments=None):
        return self.call(name, export, arguments)['value']

class Runner:
    def __init__(self, args):
        self.args = args
        self.work, self.output = args.work.resolve(), args.output.resolve()
        if self.work.exists() or self.output.exists():
            raise ValueError('work and output must be new directories')
        self.work.mkdir(parents=True)
        self.output.mkdir(parents=True)
        self.children, self.logs = [], []
        self.report = {'schema': 'fluxr-unified-executable-qualification-v1', 'passed': False,
                       'language_version': 1, 'commands': [], 'checks': [], 'applications': [],
                       'scope': 'Finite real executable, VM, native host and PostgreSQL integration cases. No speed or universal API claim.',
                       'started_at_unix': time.time(), 'sources_before': self.sources()}
    def sources(self):
        paths = [*sorted(FIXTURES.glob('*.rs')), Path(__file__), SOURCE / 'tools/fluxr.py',
                 SOURCE / 'src/runtime.rs', SOURCE / 'src/structured/source.rs',
                 SOURCE / 'src/structured/agent.rs', SOURCE / 'src/structured/unified_source.rs',
                 SOURCE / 'crates/fluxr-entry/src/lib.rs']
        return [identity(path) for path in paths if path.is_file()]
    def save(self):
        (self.output / 'report.json').write_text(json.dumps(self.report, indent=2) + '\n')
    def check(self, name, action):
        started = time.monotonic()
        row = {'name': name, 'passed': False}
        self.report['checks'].append(row)
        try:
            row['observation'] = action()
            row['passed'] = True
        except BaseException as error:
            row['error'] = f'{type(error).__name__}: {error}'
            raise
        finally:
            row['wall_seconds'] = time.monotonic() - started
            self.save()
    def run(self, label, command, *, cwd=None, env=None, expected=0, timeout=600):
        start = time.monotonic()
        command = [str(item) for item in command]
        with (self.output / (label + '.stdout.log')).open('wb') as stdout, (self.output / (label + '.stderr.log')).open('wb') as stderr:
            completed = subprocess.run(command, cwd=cwd, env=env, stdout=stdout, stderr=stderr, timeout=timeout)
        row = {'name': label, 'argv': command, 'cwd': str(cwd) if cwd else None,
               'exit_code': completed.returncode, 'expected_exit': expected,
               'wall_seconds': time.monotonic() - start,
               'stdout': identity(self.output / (label + '.stdout.log')),
               'stderr': identity(self.output / (label + '.stderr.log'))}
        self.report['commands'].append(row); self.save()
        if expected == 0:
            assert completed.returncode == 0, row
        else:
            assert completed.returncode != 0, row
        return (self.output / (label + '.stdout.log')).read_text()
    def build_fixture(self, name, native_path, *, host=False):
        root = self.work / (name + '-source'); root.mkdir()
        declaration = ('pub embedded_marker 0 int ret 314159\n'
                       'pub decimal_affine 1 x float float ret add mul x 0.5f 1.25f\n'
                       'pub decimal_negative_zero 0 float ret -0f\n')
        if host: declaration += 'pub bundled_host 1 x uint uint ret host atomic_add uint 1 uint x\n'
        authored = source_bytes(native_path.read_bytes(), declaration)
        (root / 'main.fluxr').write_bytes(authored)
        package = 'fluxr-unified-' + name
        (root / 'Cargo.toml').write_text('[package]\nname = "' + package + '"\nversion = "0.1.0"\nedition = "2024"\n'
            '[profile.release]\nopt-level = 3\nlto = "fat"\ncodegen-units = 1\npanic = "unwind"\n')
        (root / 'Cargo.lock').write_text(f'version = 4\n\n[[package]]\nname = "{package}"\nversion = "0.1.0"\n')
        files = [{'source': file, 'output': file, 'mode': 'copy', 'executable': False} for file in ('Cargo.toml', 'Cargo.lock')]
        files.append({'source': 'main.fluxr', 'output': 'src/main.rs', 'mode': 'source', 'executable': False})
        manifest = root / 'project.json'
        manifest.write_text(json.dumps({'format': 'fluxr-project-v1', 'files': files, 'rustflags': ['-C', 'target-cpu=native']}, indent=2) + '\n')
        emitted = self.work / (name + '-emitted')
        command = ([self.args.cli, 'build'] if self.args.cli else [sys.executable, '-B', SOURCE / 'tools/fluxr.py', 'build'])
        command += [manifest, emitted, '--target-dir', self.args.target_dir.resolve(), '--logs', self.output / (name + '-cargo')]
        if self.args.cli: command += ['--compiler', self.args.cli]
        self.run('build-' + name, command)
        binary = self.args.target_dir.resolve() / 'release' / package
        assert binary.is_file(), binary
        # Preserve one immutable executable per qualification arm even when
        # another build uses the same serialized Cargo cache.
        copied = self.work / package
        copied.write_bytes(binary.read_bytes()); copied.chmod(0o755)
        receipt = json.loads((emitted / 'fluxr-receipt.json').read_text())
        assert receipt.get('runtime_required') is True and receipt.get('language_version') == 1, receipt
        row = {'name': name, 'binary': identity(copied), 'authored_source': identity(root / 'main.fluxr'),
               'manifest': identity(manifest), 'emission': receipt, 'panic_profile': 'unwind'}
        self.report['applications'].append(row); self.save()
        return copied, authored.decode()
    def application(self, binary, label, *, extra=None):
        token = secrets.token_hex(32)
        discovery = self.work / (label + '-control.json')
        empty_path = self.work / 'no-compiler-bin'; empty_path.mkdir(exist_ok=True)
        env = os.environ.copy()
        removed = []
        for key in list(env):
            if key.startswith(('CARGO', 'RUST', 'FLUXR_CONTROL_')):
                removed.append(key); env.pop(key)
        env.update({'PATH': str(empty_path), 'FLUXR_CONTROL_ADDR': '127.0.0.1:0',
                    'FLUXR_CONTROL_TOKEN': token, 'FLUXR_CONTROL_FILE': str(discovery)})
        env.update(extra or {})
        out = (self.output / (label + '-process.stdout.log')).open('wb')
        err = (self.output / (label + '-process.stderr.log')).open('wb')
        self.logs.extend([out, err])
        process = subprocess.Popen([str(binary)], env=env, stdin=subprocess.PIPE, stdout=out, stderr=err, start_new_session=True)
        self.children.append(process)
        deadline = time.monotonic() + 15
        while not discovery.exists():
            if process.poll() is not None: raise RuntimeError(label + ' exited before control discovery')
            if time.monotonic() >= deadline: raise TimeoutError(label + ' control startup')
            time.sleep(.02)
        info = json.loads(discovery.read_text())
        assert info['version'] == 1 and info['language_version'] == 1 and info['pid'] == process.pid, info
        assert 'token' not in info, info
        control = Control(info['address'], token)
        # Discovery is written before original main. Wait for native fixture
        # readiness so registered host services have a deterministic start.
        ready = b'UNIFIED_HOST_READY' if label == 'host' else b'UNIFIED_MINIMAL_READY'
        while ready not in (self.output / (label + '-process.stdout.log')).read_bytes():
            if process.poll() is not None: raise RuntimeError(label + ' exited before native main')
            if time.monotonic() >= deadline: raise TimeoutError(label + ' native readiness')
            time.sleep(.02)
        self.report.setdefault('runtime_environments', []).append({'label': label, 'pid': process.pid,
            'path': env['PATH'], 'removed_variables': sorted(removed), 'compiler_directory_empty': not any(empty_path.iterdir()),
            'discovery': info, 'token_recorded': False})
        self.save()
        return process, control
    def stop_application(self, process):
        assert process.poll() is None, 'application exited before intentional stop'
        process.stdin.write(b'stop\n'); process.stdin.flush(); process.stdin.close()
        assert process.wait(timeout=10) == 0
    def postgres(self):
        pg_bin = Path(os.environ['PG_BIN']); data = self.work / 'postgres-cluster'
        self.run('pg-init', [pg_bin / 'initdb', '-D', data, '-L', os.environ['PG_SHARE'], '-A', 'trust', '-U', 'unified_test', '--no-locale', '--encoding=UTF8'])
        port = free_port()
        log = (self.output / 'postgres.log').open('wb'); self.logs.append(log)
        command = [str(pg_bin / 'postgres'), '-D', str(data), '-h', '127.0.0.1', '-p', str(port),
                   '-c', 'unix_socket_directories=', '-c', 'fsync=on', '-c', 'synchronous_commit=on', '-c', 'max_connections=24']
        process = subprocess.Popen(command, stdout=log, stderr=log, start_new_session=True)
        self.children.append(process)
        pg_env = os.environ | {'PGHOST': '127.0.0.1', 'PGPORT': str(port), 'PGUSER': 'unified_test', 'PGDATABASE': 'postgres'}
        deadline = time.monotonic() + 15
        while subprocess.run([str(pg_bin / 'pg_isready')], env=pg_env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode != 0:
            if process.poll() is not None or time.monotonic() >= deadline: raise RuntimeError('PostgreSQL startup failed')
            time.sleep(.05)
        psql = [pg_bin / 'psql', '-X', '-v', 'ON_ERROR_STOP=1', '-At', '-c']
        self.run('pg-setup', psql + ['CREATE TABLE unified_probe(id integer PRIMARY KEY, n bigint NOT NULL); INSERT INTO unified_probe VALUES(1,0);'], env=pg_env)
        settings = self.run('pg-settings', psql + ['SHOW fsync; SHOW synchronous_commit; SELECT version();'], env=pg_env)
        assert settings.splitlines()[:2] == ['on', 'on'], settings
        self.report['postgres'] = {'argv': command, 'pid': process.pid, 'settings': settings}
        host_env = {'FLUXR_TEST_PG_HOST': '127.0.0.1', 'FLUXR_TEST_PG_PORT': str(port),
                    'FLUXR_TEST_PG_USER': 'unified_test', 'FLUXR_TEST_PG_DATABASE': 'postgres',
                    'FLUXR_TEST_UNIX_PATH': str(self.work / 'probe.sock')}
        return process, host_env, psql, pg_env
    def minimal(self, binary, authored):
        process, control = self.application(binary, 'minimal')
        def bootstrap():
            modules = control.okay({'op': 'list'})
            assert any(m['name'] == 'main.fluxr' and m['revision'] == 1 for m in modules), modules
            result = control.call('main.fluxr', 'embedded_marker')
            assert result['value'] == {'i64': 314159}, result
            decimal_affine = control.call('main.fluxr', 'decimal_affine', [{'f32': 0x40000000}])
            assert decimal_affine['value'] == {'f32': 0x40100000}, decimal_affine
            decimal_negative_zero = control.call('main.fluxr', 'decimal_negative_zero')
            assert decimal_negative_zero['value'] == {'f32': 0x80000000}, decimal_negative_zero
            return {'pid': process.pid, 'bundled_call': result,
                    'decimal_affine_call': decimal_affine, 'decimal_negative_zero_call': decimal_negative_zero,
                    'native_main_has_no_runtime_api_calls': True}
        self.check('automatic-runtime-bootstrap-before-native-main', bootstrap)
        def injection():
            first = control.install('live', 'fluxr 1\npub f 1 x int int ret add x 3\n')
            assert first['revision'] == 1
            assert control.value('live', 'f', [{'i64': 9}]) == {'i64': 12}
            second = control.install('live', 'fluxr 1\npub f 1 x int int ret mul x 5\n', 1)
            assert second['revision'] == 2
            assert control.value('live', 'f', [{'i64': 9}]) == {'i64': 45}
            return {'first_revision': 1, 'second_revision': 2, 'same_pid': process.poll() is None}
        self.check('compiler-free-create-call-cas-update', injection)
        def rejected_publication():
            command = {'op': 'install', 'name': 'live', 'expected_revision': 1, 'source': 'fluxr 1\npub f 1 x int int ret 99\n'}
            control.rejected(command, 'conflict')
            command['expected_revision'] = 2; command['source'] = 'fluxr 1\npub f 1 x int int ret true\n'
            control.rejected(command)
            command['source'] = 'fluxr 1\npub f 0 bool ret true\n'; control.rejected(command, 'interface')
            assert control.call('live', 'f', [{'i64': 9}]) == {'revision': 2, 'value': {'i64': 45}, 'route': 'vm'}
            return {'active_revision': 2, 'body_preserved_after_stale_type_and_interface_errors': True}
        self.check('failed-publications-preserve-active-revision', rejected_publication)
        def admission():
            cases = [('legacy', 'fa1\npub f 0 int ret 1\n'), ('future', 'fluxr 2\npub f 0 int ret 1\n'),
                     ('missing_host', 'fluxr 1\npub f 0 unit ret host unavailable unit 0\n')]
            errors = {}
            for name, text in cases:
                errors[name] = control.rejected({'op': 'install', 'name': name, 'source': text, 'expected_revision': None})
            native = source_bytes(b'fn main() {}\n', '').decode()
            errors['unbound_native'] = control.rejected({'op': 'install', 'name': 'unbound', 'source': native, 'expected_revision': None}, 'native_unbound')
            changed = authored.replace('UNIFIED_MINIMAL_READY', 'UNIFIED_MINIMAL_OTHER')
            errors['changed_native'] = control.rejected({'op': 'install', 'name': 'main.fluxr', 'source': changed, 'expected_revision': 1}, 'native_identity')
            return errors
        self.check('canonical-source-and-build-identity-admission', admission)
        def bundled_replace():
            revised = authored.replace('ret 314159\n', 'ret 271828\n', 1)
            info = control.install('main.fluxr', revised, 1)
            result = control.call('main.fluxr', 'embedded_marker')
            assert info['revision'] == 2 and result['value'] == {'i64': 271828}, result
            return {'revision': 2, 'native_digest': info['native_digest'], 'route': result['route']}
        self.check('bundled-checked-code-updates-with-same-native-binding', bundled_replace)
        def protocol():
            bad = control.request({'op': 'list'}, token='incorrect-token')
            assert bad['ok'] is False and bad['error']['code'] == 'authentication', bad
            bad = control.request({'op': 'list'}, version=2)
            assert bad['ok'] is False and bad['error']['code'] == 'version', bad
            bad = exchange(control.address, b'{"truncated":}\n')
            assert bad['ok'] is False and bad['error']['code'] == 'invalid_request', bad
            unknown = {'version': 1, 'token': control.token, 'command': {'op': 'list', 'unexpected': 1}}
            bad = exchange(control.address, json_bytes(unknown))
            assert bad['ok'] is False and bad['error']['code'] == 'invalid_request', bad
            oversized = None
            try:
                oversized = exchange(control.address, b'x' * (MAX_WIRE + 1) + b'\n')
                assert oversized['ok'] is False and oversized['error']['code'] == 'input_limit', oversized
            except (ConnectionError, OSError):
                oversized = {'connection_closed': True}
            assert control.value('live', 'f', [{'i64': 2}]) == {'i64': 10}
            return {'bad_auth_version_json_unknown_fields_rejected': True, 'oversized': oversized, 'process_alive': process.poll() is None}
        self.check('authenticated-bounded-control-negative-cases', protocol)
        def limits():
            errors = []
            for budget in ({'instructions': 0, 'cells': 16, 'frames': 8}, {'instructions': 1_000_000_001, 'cells': 16, 'frames': 8}):
                errors.append(control.rejected({'op': 'call', 'name': 'live', 'export': 'f', 'arguments': [{'i64': 2}], 'limits': budget}))
            return errors
        self.check('vm-budget-and-runtime-ceiling-enforced', limits)
        if self.args.cli:
            def primary_cli():
                file = self.work / 'cli-injected.fluxr'
                file.write_bytes(b'fluxr 1\npub f 0 int ret 8080\n')
                env = os.environ | {'FLUXR_CONTROL_TOKEN': control.token}
                installed = json.loads(self.run('cli-install', [self.args.cli, 'install', control.address, 'cli_module', 'new', file], env=env))
                called = json.loads(self.run('cli-call', [self.args.cli, 'call', control.address, 'cli_module', 'f'], env=env))
                assert installed['ok'] is True and called['result']['value'] == {'i64': 8080}, called
                return {'same_process_pid': process.pid, 'call': called['result']}
            self.check('primary-cli-installs-and-calls-live-compiled-process', primary_cli)
        self.stop_application(process)
        self.check('startup-rejects-invalid-control-configuration', lambda: self.startup_rejections(binary))
    def startup_rejections(self, binary):
        cases = [('missing-token', {'FLUXR_CONTROL_ADDR': '127.0.0.1:0'}),
                 ('short-token', {'FLUXR_CONTROL_ADDR': '127.0.0.1:0', 'FLUXR_CONTROL_TOKEN': 'short'}),
                 ('public-bind', {'FLUXR_CONTROL_ADDR': '0.0.0.0:0', 'FLUXR_CONTROL_TOKEN': 'a' * 64})]
        results = []
        for label, configuration in cases:
            env = {k: v for k, v in os.environ.items() if not k.startswith('FLUXR_CONTROL_')}
            env.update(configuration)
            completed = subprocess.run([str(binary)], env=env, input=b'stop\n', capture_output=True, timeout=10)
            (self.output / ('startup-' + label + '.stdout.log')).write_bytes(completed.stdout)
            (self.output / ('startup-' + label + '.stderr.log')).write_bytes(completed.stderr)
            assert completed.returncode != 0 and b'UNIFIED_MINIMAL_READY' not in completed.stdout, (label, completed)
            results.append({'case': label, 'exit_code': completed.returncode})
        return results
    def target_admission(self):
        root = self.work / 'alternate-source'; root.mkdir()
        native = (FIXTURES / 'minimal_fixture.rs').read_bytes()
        package = 'fluxr-unified-alternate'
        cargo = ('[package]\nname = "' + package + '"\nversion = "0.1.0"\nedition = "2024"\n'
                 'build = "tools/make.rs"\n[[bin]]\nname = "custom-entry"\npath = "app/entry.rs"\n')
        (root / 'Cargo.toml').write_text(cargo)
        (root / 'Cargo.lock').write_text(f'version = 4\n\n[[package]]\nname = "{package}"\nversion = "0.1.0"\n')
        (root / 'custom.fluxr').write_bytes(source_bytes(native))
        (root / 'automatic.fluxr').write_bytes(source_bytes(native))
        build_body = b'fn main() { println!("cargo:rustc-env=UNIFIED_BUILD_SCRIPT_RAN=yes"); }\n'
        (root / 'build.fluxr').write_bytes(source_bytes(build_body, ''))
        files = [{'source': name, 'output': name, 'mode': 'copy', 'executable': False} for name in ('Cargo.toml', 'Cargo.lock')]
        for authored, destination in [('custom.fluxr', 'app/entry.rs'), ('automatic.fluxr', 'src/bin/automatic-entry.rs'), ('build.fluxr', 'tools/make.rs')]:
            files.append({'source': authored, 'output': destination, 'mode': 'source', 'executable': False})
        manifest = root / 'project.json'
        configuration = {'format': 'fluxr-project-v1', 'files': files, 'rustflags': []}
        manifest.write_text(json.dumps(configuration, indent=2) + '\n')
        emitted = self.work / 'alternate-emitted'
        command = ([self.args.cli, 'build'] if self.args.cli else [sys.executable, '-B', SOURCE / 'tools/fluxr.py', 'build'])
        command += [manifest, emitted, '--target-dir', self.args.target_dir.resolve(), '--logs', self.output / 'alternate-cargo']
        if self.args.cli: command += ['--compiler', self.args.cli]
        self.run('build-alternate-targets', command)
        receipt = json.loads((emitted / 'fluxr-receipt.json').read_text())
        assert receipt['cargo_roles']['tools/make.rs'] == 'build-script'
        assert receipt['cargo_roles']['app/entry.rs'] == 'binary'
        assert receipt['cargo_roles']['src/bin/automatic-entry.rs'] == 'binary'
        observations = []
        for binary_name, source_name in [('custom-entry', 'custom.fluxr'), ('automatic-entry', 'automatic.fluxr')]:
            binary = self.args.target_dir.resolve() / 'release' / binary_name
            before = identity(binary)
            process, control = self.application(binary, binary_name)
            assert control.value(source_name, 'embedded_marker') == {'i64': 314159}
            listed = control.okay({'op': 'list'})
            assert {row['name'] for row in listed} == {'custom.fluxr', 'automatic.fluxr'}, listed
            control.install('live', 'fluxr 1\npub f 0 int ret 77\n')
            assert control.value('live', 'f') == {'i64': 77}
            self.stop_application(process)
            assert before == identity(binary)
            observations.append({'target': binary_name, 'source': source_name, 'pid': process.pid, 'binary': before})
        rejections = []
        # Mutations are isolated fixture inputs. Every negative command must
        # reject before export and leave its requested destination absent.
        for name in ('ambiguous-role', 'checked-build-script', 'runtime-override', 'raw-source-copy', 'legacy-manifest'):
            (root / 'Cargo.toml').write_text(cargo)
            (root / 'build.fluxr').write_bytes(source_bytes(build_body, ''))
            candidate = json.loads(json.dumps(configuration))
            if name == 'ambiguous-role':
                (root / 'Cargo.toml').write_text(cargo.replace('build = "tools/make.rs"', 'build = "app/entry.rs"'))
            elif name == 'checked-build-script':
                (root / 'build.fluxr').write_bytes(source_bytes(build_body))
            elif name == 'runtime-override':
                (root / 'Cargo.toml').write_text(cargo + '\n[dependencies.fluxr]\npath = "unapproved-runtime"\n')
            elif name == 'raw-source-copy':
                candidate['files'][2]['mode'] = 'copy'
            elif name == 'legacy-manifest':
                candidate['format'] = 'fluxr-native-project-v1'
            manifest.write_text(json.dumps(candidate) + '\n')
            destination = self.work / ('must-not-exist-' + name)
            self.run('reject-target-' + name, [sys.executable, '-B', SOURCE / 'tools/fluxr.py', 'export', manifest, destination], expected=1)
            assert not destination.exists(), name
            rejections.append(name)
        return {'compiled_and_injected_targets': observations, 'rejected_admissions': rejections,
                'foreign_build_script_excluded_from_application_catalog': True}
    def host(self, binary, host_env, psql, pg_env):
        process, control = self.application(binary, 'host', extra=host_env)
        source = '''fluxr 1
pub add 1 n uint uint ret host atomic_add uint 1 uint n
pub read 0 uint ret host atomic_read uint 0
pub threads 1 n uint uint ret host thread_sum uint 1 uint n
pub tcp 1 n uint uint ret host tcp_roundtrip uint 1 uint n
pub pg 1 n uint uint ret host pg_add uint 1 uint n
pub rollback 1 n uint uint ret host pg_rollback uint 1 uint n
pub readers 0 list uint ret host reader_stats list uint 0
pub pin 0 uint ret host pin_hot uint 0
pub pinned 0 int ret host call_pinned int 0
pub release 0 unit ret host release_pin unit 0
pub reentrant 1 n uint uint ret host reentrant_install uint 1 uint n
pub error 0 unit ret host host_error unit 0
pub panic 0 unit ret host host_panic unit 0
pub wrong 0 uint ret host wrong_result uint 0
pub unix 0 int ret host unix_listener int 0
'''
        control.install('services', source)
        def bundled_host():
            assert control.value('main.fluxr', 'bundled_host', [{'u64': 5}]) == {'u64': 5}
            return {'bundled_before_native_registration': True, 'registered_binding_invoked': True}
        self.check('bundled-host-requirements-bind-after-runtime-bootstrap', bundled_host)
        def atomics():
            with ThreadPoolExecutor(max_workers=4) as pool:
                values = list(pool.map(lambda _: control.value('services', 'add', [{'u64': 1}])['u64'], range(128)))
            assert sorted(values) == list(range(6, 134)), values
            assert control.value('services', 'read') == {'u64': 133}
            return {'concurrent_calls': 128, 'observed_exact_contiguous_atomic_returns': True, 'final_value': 133}
        self.check('injected-atomic-host-calls-linearize', atomics)
        def threads_network():
            for n in (0, 1, 7, 10000, 200000):
                assert control.value('services', 'threads', [{'u64': n}]) == {'u64': n * (n + 1) // 2}
            for n in (0, 1, (1 << 63), (1 << 64) - 1):
                rotated = ((n << 13) | (n >> 51)) & ((1 << 64) - 1)
                assert control.value('services', 'tcp', [{'u64': n}]) == {'u64': rotated ^ 0x55aa0123456789ef}
            return {'thread_cases': 5, 'native_threads_per_call': 4, 'tcp_cases': 4, 'fragmentation_and_half_close': True}
        self.check('injected-native-threads-and-real-tcp', threads_network)
        def database():
            with ThreadPoolExecutor(max_workers=4) as pool:
                values = list(pool.map(lambda _: control.value('services', 'pg', [{'u64': 1}])['u64'], range(16)))
            assert sorted(values) == list(range(1, 17)), values
            assert control.value('services', 'rollback', [{'u64': 999}]) == {'u64': 16}
            oracle = self.run('pg-independent-oracle', psql + ['SELECT n FROM unified_probe WHERE id=1'], env=pg_env).strip()
            assert oracle == '16', oracle
            return {'concurrent_committed_transactions': 16, 'rollback_preserved_value': 16, 'independent_psql_value': 16}
        self.check('injected-real-postgres-transactions-concurrency-and-rollback', database)
        def revisions():
            assert control.value('services', 'pin') == {'u64': 1}
            def reads(_):
                results = []
                for _ in range(120):
                    result = control.call('hot', 'f')
                    assert result['value'] == {'i64': result['revision'] * 17 + 3}, result
                    results.append(result['revision'])
                return results
            with ThreadPoolExecutor(max_workers=4) as pool:
                futures = [pool.submit(reads, lane) for lane in range(3)]
                for revision in range(2, 62):
                    result = control.install('hot', f'fluxr 1\npub f 0 int ret {revision * 17 + 3}\n', revision - 1)
                    assert result['revision'] == revision, result
                observed = [revision for future in futures for revision in future.result()]
            assert control.value('services', 'pinned') == {'i64': 20}
            assert control.value('services', 'release') == 'unit'
            stats = control.value('services', 'readers')['list']
            assert stats[0]['u64'] > 0 and stats[1]['u64'] == 0, stats
            return {'successful_replacements': 60, 'control_reads': len(observed), 'revision_range_seen': [min(observed), max(observed)],
                    'native_reader_calls': stats[0]['u64'], 'native_reader_mixed_revisions': stats[1]['u64'],
                    'old_snapshot_value_after_60_replacements': 20}
        self.check('concurrent-live-revisions-and-retained-snapshots', revisions)
        def callbacks():
            for n in (123, 456):
                assert control.value('services', 'reentrant', [{'u64': n}]) == {'u64': n}
            errors = {}
            for export in ('error', 'panic', 'wrong'):
                errors[export] = control.rejected({'op': 'call', 'name': 'services', 'export': export}, 'execution')
                assert control.value('services', 'read') == {'u64': 133}
            control.rejected({'op': 'call', 'name': 'services', 'export': 'add', 'arguments': [{'i64': 1}]})
            assert control.value('services', 'read') == {'u64': 133}
            missing = 'fluxr 1\npub f 0 int ret host atomic_read int 0\n'
            errors['signature'] = control.rejected({'op': 'install', 'name': 'wrong_signature', 'source': missing, 'expected_revision': None}, 'host_signature')
            return {'reentrant_install_and_call': True, 'errors': errors, 'panic_scope': 'unwind build only', 'process_alive': process.poll() is None}
        self.check('reentrant-host-calls-and-error-type-panic-containment', callbacks)
        def unix_observation():
            value = control.value('services', 'unix')['i64']
            self.report['filesystem_unix_capability_passed'] = value == 0
            return {'native_os_error': value, 'required_capability_passed': value == 0,
                    'scope': 'Actual native filesystem UnixListener through injected host call; EPERM is an environmental capability failure, never converted to a capability pass.'}
        self.check('record-real-filesystem-unix-host-result', unix_observation)
        self.stop_application(process)
        log = (self.output / 'host-process.stdout.log').read_text()
        last = [line for line in log.splitlines() if line.startswith('UNIFIED_HOST_STOP ')]
        assert len(last) == 1 and last[0].split()[-1] == '0', log
    def execute(self):
        try:
            minimal, minimal_source = self.build_fixture('minimal', FIXTURES / 'minimal_fixture.rs')
            host, _ = self.build_fixture('host', FIXTURES / 'host_fixture.rs', host=True)
            self.minimal(minimal, minimal_source)
            self.check('alternate-cargo-entry-paths-and-negative-runtime-admission', self.target_admission)
            pg, host_env, psql, pg_env = self.postgres()
            self.host(host, host_env, psql, pg_env)
            pg.send_signal(signal.SIGTERM)
            assert pg.wait(timeout=20) == 0
            self.report['sources_after'] = self.sources()
            self.report['sources_unchanged'] = self.report['sources_before'] == self.report['sources_after']
            expected_checks = 17 if self.args.cli else 16
            self.report['planned_checks'] = expected_checks
            self.report['passed'] = (len(self.report['checks']) == expected_checks and all(c['passed'] for c in self.report['checks'])
                                     and self.report['sources_unchanged'])
        except BaseException as error:
            self.report['error'] = f'{type(error).__name__}: {error}'
            raise
        finally:
            cleanup = []
            for child in self.children:
                before = child.poll()
                if before is None:
                    try: os.killpg(child.pid, signal.SIGTERM)
                    except ProcessLookupError: pass
                    try: child.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        os.killpg(child.pid, signal.SIGKILL); child.wait(timeout=10)
                cleanup.append({'pid': child.pid, 'exit_before_cleanup': before, 'exit_code': child.returncode})
            for log in self.logs: log.close()
            self.report['cleanup'] = cleanup
            self.report['finished_at_unix'] = time.time()
            self.save()
        return 0 if self.report['passed'] else 1

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--target-dir', type=Path, required=True)
    parser.add_argument('--cli', type=Path)
    args = parser.parse_args()
    return Runner(args).execute()

if __name__ == '__main__':
    raise SystemExit(main())
