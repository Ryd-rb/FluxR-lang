"""Concurrent VM publication/call and HTTP probe for real challenge servers."""
from __future__ import annotations
import json
from pathlib import Path
import socket
import threading
import time
import urllib.request

class LiveProbe:
    def __init__(self, discovery: Path, token: str, process, base: str):
        self.discovery, self.token, self.process, self.base = discovery, token, process, base
        self.stop = threading.Event()
        self.lock = threading.Lock()
        self.threads = []
        self.report = {'passed': False, 'pid': process.pid, 'installations': 0,
                       'checked_calls': 0, 'http_successes': 0, 'errors': [],
                       'scope': 'In-process checked VM injection while official HTTP conformance is active; independent of request-routing policy.'}
    def request(self, command):
        payload = (json.dumps({'version': 1, 'token': self.token, 'command': command}, separators=(',', ':')) + '\n').encode()
        host, port = self.address.rsplit(':', 1)
        with socket.create_connection((host, int(port)), timeout=10) as stream:
            stream.settimeout(10); stream.sendall(payload); stream.shutdown(socket.SHUT_WR)
            with stream.makefile('rb') as reader:
                response = reader.readline(2 * 1024 * 1024 + 1)
        assert response.endswith(b'\n') and len(response) <= 2 * 1024 * 1024, 'bounded control response absent'
        value = json.loads(response)
        assert value['version'] == 1 and value['ok'] is True, value
        return value['result']
    def install(self, revision):
        source = f'fluxr 1\npub value 0 int ret {revision * 17 + 3}\n'
        result = self.request({'op': 'install', 'name': 'challenge_probe', 'expected_revision': None if revision == 1 else revision - 1, 'source': source})
        assert result['revision'] == revision, result
        with self.lock:
            self.report['installations'] += 1
            self.report['last_published_revision'] = revision
    def guarded(self, operation):
        try: operation()
        except BaseException as error:
            with self.lock: self.report['errors'].append(f'{type(error).__name__}: {error}')
            self.stop.set()
    def start(self):
        deadline = time.monotonic() + 15
        while not self.discovery.exists():
            if self.process.poll() is not None: raise RuntimeError('server exited before VM control discovery')
            if time.monotonic() >= deadline: raise TimeoutError('server VM endpoint did not start')
            time.sleep(.02)
        discovery = json.loads(self.discovery.read_text())
        assert discovery['pid'] == self.process.pid and discovery['language_version'] == 1 and discovery['version'] == 1, discovery
        assert 'token' not in discovery, discovery
        self.address = discovery['address']
        self.report['discovery'] = discovery
        self.report['original_modules'] = self.request({'op': 'list'})
        self.install(1)
        def write():
            revision = 2
            while not self.stop.is_set() and revision <= 2000:
                self.install(revision); revision += 1
                self.stop.wait(.005)
        def read():
            while not self.stop.is_set():
                result = self.request({'op': 'call', 'name': 'challenge_probe', 'export': 'value'})
                assert result['value'] == {'i64': result['revision'] * 17 + 3}, result
                with self.lock:
                    self.report['checked_calls'] += 1
                    self.report['max_observed_revision'] = max(result['revision'], self.report.get('max_observed_revision', 0))
                self.stop.wait(.003)
        def http():
            while not self.stop.is_set():
                with urllib.request.urlopen(self.base + '/api/tags', timeout=5) as response:
                    assert response.status == 200
                    body = json.loads(response.read(2 * 1024 * 1024))
                    assert isinstance(body.get('tags'), list), body
                with self.lock: self.report['http_successes'] += 1
                self.stop.wait(.005)
        self.report['started_at_unix'] = time.time()
        for name, action in [('publisher', write), ('reader', read), ('http', http)]:
            worker = threading.Thread(name='fluxr-' + name, target=self.guarded, args=(action,), daemon=True)
            self.threads.append(worker); worker.start()
        return self
    def mark_conformance_start(self):
        with self.lock:
            self.report['counts_at_conformance_start'] = {key: self.report[key] for key in ('installations', 'checked_calls', 'http_successes')}
        self.report['conformance_started_at_unix'] = time.time()
    def finish(self):
        self.stop.set()
        for worker in self.threads: worker.join(timeout=15)
        self.report['threads_joined'] = all(not worker.is_alive() for worker in self.threads)
        self.report['same_process_alive'] = self.process.poll() is None
        self.report['finished_at_unix'] = time.time()
        initial = self.report.get('counts_at_conformance_start', {})
        self.report['during_conformance'] = {key: self.report[key] - initial.get(key, self.report[key]) for key in ('installations', 'checked_calls', 'http_successes')}
        self.report['passed'] = (not self.report['errors'] and self.report['threads_joined'] and self.report['same_process_alive']
                                and self.report['installations'] >= 2 and self.report['checked_calls'] >= 2
                                and self.report['http_successes'] >= 2 and self.report.get('max_observed_revision', 0) >= 2
                                and all(value >= 1 for value in self.report['during_conformance'].values()))
        return self.report
