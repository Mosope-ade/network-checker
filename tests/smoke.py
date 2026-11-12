"""End-to-end API checks against a fresh, isolated database. Run after cargo build."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import unittest
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]

class MonitorTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix='netcheck-smoke-')
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            cls.port = sock.getsockname()[1]
        cls.base = f'http://127.0.0.1:{cls.port}'
        cls.db = os.path.join(cls.temp.name, 'monitor.db')
        cls.start_server()

    @classmethod
    def start_server(cls):
        cls.process = subprocess.Popen([str(ROOT / 'target/debug/netcheck'), '--port', str(cls.port), '--database', cls.db], stdout=subprocess.DEVNULL)
        for _ in range(100):
            try:
                cls.request('/api/status')
                return
            except (OSError, urllib.error.URLError):
                time.sleep(.1)
        raise RuntimeError('Server did not start')

    @classmethod
    def tearDownClass(cls):
        cls.process.terminate()
        cls.process.wait(timeout=10)
        cls.temp.cleanup()

    @classmethod
    def request(cls, path, data=None, method=None, headers=None):
        req = urllib.request.Request(cls.base + path, data=json.dumps(data).encode() if data is not None else None,
                                     method=method, headers={'Content-Type':'application/json', **(headers or {})})
        with urllib.request.urlopen(req, timeout=5) as res:
            body = res.read()
            return json.loads(body) if 'application/json' in res.headers.get('Content-Type', '') else body

    def wait_for(self, predicate, timeout=15):
        until = time.monotonic() + timeout
        while time.monotonic() < until:
            status = self.request('/api/status')
            if predicate(status): return status
            time.sleep(.2)
        self.fail(f'Timed out waiting for monitoring state: {status}')

    def test_complete_workflow(self):
        self.assertEqual(self.request('/api/status')['targets'], [])
        config = dict(interval=1, failure_threshold=2, recovery_threshold=2)
        for ip, name in [('127.0.0.1','IPv4 loopback'), ('::1','IPv6 loopback'), ('192.0.2.1','=Unreachable')]:
            self.request('/api/targets', dict(config, address=ip, name=name))
        status = self.wait_for(lambda s: len(s['incidents']) == 1 and all(t['sent'] >= 2 for t in s['targets']))
        self.assertEqual([t['status'] for t in status['targets']], ['up','up','down'])
        # An unreachable target must not delay healthy probes on other targets.
        self.assertGreater(status['targets'][0]['sent'], status['targets'][2]['sent'])
        for data in [dict(config,address='garbage',name='invalid'), dict(config,address='127.0.0.1',name='duplicate')]:
            with self.assertRaises(urllib.error.HTTPError) as error: self.request('/api/targets', data)
            self.assertIn(error.exception.code, [400,409])
            error.exception.close()
        with self.assertRaises(urllib.error.HTTPError) as error:
            self.request('/api/targets/1/toggle', method='POST', headers={'Origin':'http://evil.example'})
        self.assertEqual(error.exception.code,403)
        error.exception.close()
        with self.assertRaises(urllib.error.HTTPError) as error:
            self.request('/api/status', headers={'Host':'evil.example'})
        self.assertEqual(error.exception.code,403)
        error.exception.close()
        self.request('/api/targets/3/toggle', method='POST')
        state = self.request('/api/status')
        self.assertEqual(state['targets'][2]['status'], 'paused')
        self.assertEqual(state['incidents'][0]['outcome'], 'interrupted')
        sent = state['targets'][2]['sent']
        time.sleep(2.5)
        self.assertEqual(self.request('/api/status')['targets'][2]['sent'], sent)
        self.request('/api/targets/3/toggle', method='POST')
        self.wait_for(lambda s: len(s['incidents']) == 2)
        self.process.terminate(); self.process.wait(timeout=10)
        self.start_server()
        state = self.request('/api/status')
        self.assertEqual(len(state['targets']), 3)
        self.assertEqual(state['incidents'][0]['outcome'], 'interrupted')
        self.request('/api/targets/3', method='DELETE')
        self.assertEqual(len(self.request('/api/status')['targets']),2)
        csv = self.request('/api/incidents.csv').decode()
        self.assertIn("'=Unreachable",csv)
        self.assertIn('interrupted', csv)
        print('Verified concurrent IPv4/IPv6 ping, timeout, pause/resume, restart, retained logs, CSV, validation and origin protection.')

if __name__ == '__main__': unittest.main()
