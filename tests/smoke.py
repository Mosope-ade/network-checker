"""Real-ping API regression checks with isolated storage and caller-supplied failure target."""
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import time
import unittest
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
LOOPBACK = socket.gethostbyname('localhost')
IPV6_LOOPBACK = socket.getaddrinfo('localhost', None, socket.AF_INET6)[0][4][0]

class MonitorTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='netcheck-smoke-')
        with socket.socket() as sock:
            sock.bind(('localhost', 0))
            self.port = sock.getsockname()[1]
        self.base = f'http://localhost:{self.port}'
        self.db = os.path.join(self.temp.name, 'monitor.db')
        self.start_server()

    def start_server(self):
        self.process = subprocess.Popen([str(ROOT / 'target/debug/netcheck'), '--port', str(self.port), '--database', self.db], stdout=subprocess.DEVNULL)
        for _ in range(100):
            try:
                self.request('/api/status')
                return
            except (OSError, urllib.error.URLError):
                if self.process.poll() is not None: raise RuntimeError('Server exited during startup')
                time.sleep(.1)
        self.process.terminate(); self.process.wait(timeout=10)
        raise RuntimeError('Server did not start')

    def tearDown(self):
        self.process.terminate()
        self.process.wait(timeout=10)
        self.temp.cleanup()

    def request(self, path, data=None, method=None, headers=None):
        req = urllib.request.Request(self.base + path, data=json.dumps(data).encode() if data is not None else None,
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

    def expect_error(self, code, *args, **kwargs):
        with self.assertRaises(urllib.error.HTTPError) as error: self.request(*args, **kwargs)
        self.assertEqual(error.exception.code,code)
        error.exception.close()

    def test_no_preset_targets_and_removed_configuration_path(self):
        self.assertEqual(self.request('/api/status')['targets'], [])
        for flag in ['--daemon','--config']:
            result = subprocess.run([str(ROOT/'target/debug/netcheck'),flag],capture_output=True,timeout=5)
            self.assertEqual(result.returncode,2)

    def test_errors_are_clear_json_messages(self):
        for path,data,method,expected_code in [
            ('/api/targets',{},'POST',422),
            ('/api/targets',{'name':'router','address':'invalid','interval':1,'failure_threshold':1,'recovery_threshold':1},'POST',400),
            ('/api/targets/999/state',{'enabled':False},'PUT',404),
            ('/api/targets/999/state',{'enabled':'wrong'},'PUT',422),
            ('/api/status',None,'POST',405),
            ('/api/does-not-exist',None,'GET',404),
            ('/api/targets',{'name':'x'*20000},'POST',413),
        ]:
            with self.assertRaises(urllib.error.HTTPError) as error:
                self.request(path,data,method=method)
            with error.exception as response:
                self.assertEqual(response.code,expected_code)
                self.assertIn('application/json',response.headers['Content-Type'])
                body=json.load(response)
                self.assertEqual(body['error']['code'],expected_code)
                self.assertTrue(body['error']['message'])
                self.assertNotIn('deserialize',body['error']['message'])
        malformed=urllib.request.Request(self.base+'/api/targets',data=b'{',headers={'Content-Type':'application/json'})
        with self.assertRaises(urllib.error.HTTPError) as error: urllib.request.urlopen(malformed,timeout=5)
        with error.exception as response:
            self.assertEqual(response.code,400)
            self.assertNotIn('deserialize',json.load(response)['error']['message'])
        with sqlite3.connect(self.db) as connection: connection.execute('DROP TABLE incidents')
        with self.assertRaises(urllib.error.HTTPError) as error: self.request('/api/status')
        with error.exception as response:
            body=json.load(response)
            self.assertEqual(response.code,500)
            self.assertNotIn('incidents',body['error']['message'])
            self.assertNotIn('SQL',body['error']['message'])

    def test_complete_workflow(self):
        unreachable = os.environ.get('NETCHECK_TEST_UNREACHABLE')
        if not unreachable: self.skipTest('Set NETCHECK_TEST_UNREACHABLE to a host IP you know will not reply')
        config = dict(interval=1, failure_threshold=2, recovery_threshold=2)
        for ip, name in [(LOOPBACK,'IPv4 loopback'), (IPV6_LOOPBACK,'IPv6 loopback'), (unreachable,'=Unreachable')]:
            self.request('/api/targets', dict(config, address=ip, name=name))
        status = self.wait_for(lambda s: len(s['incidents']) == 1 and all(t['sent'] >= 2 for t in s['targets']))
        self.assertEqual([t['status'] for t in status['targets']], ['up','up','down'])
        self.assertGreater(status['targets'][0]['sent'], status['targets'][2]['sent'])
        self.expect_error(400,'/api/targets',dict(config,address='garbage',name='invalid'))
        self.expect_error(409,'/api/targets',dict(config,address=LOOPBACK,name='duplicate'))
        self.expect_error(403,'/api/targets/1/state',dict(enabled=False),method='PUT',headers={'Origin':'http://evil.example'})
        self.expect_error(403,'/api/status',headers={'Host':'evil.example'})
        self.expect_error(404,'/api/targets/999/state',dict(enabled=False),method='PUT')
        self.expect_error(400,'/api/targets/not-a-number',method='DELETE')
        self.request('/api/targets/3/state',dict(enabled=False),method='PUT')
        state = self.request('/api/status')
        self.assertEqual(state['targets'][2]['status'], 'paused')
        self.assertEqual(state['incidents'][0]['outcome'], 'interrupted')
        sent = state['targets'][2]['sent']
        generation = state['targets'][2]['generation']
        # Repeating an explicit state change is safe; it must not toggle monitoring back on.
        self.request('/api/targets/3/state',dict(enabled=False),method='PUT')
        time.sleep(2.5)
        self.assertEqual(self.request('/api/status')['targets'][2]['sent'], sent)
        self.assertEqual(self.request('/api/status')['targets'][2]['generation'],generation)
        self.request('/api/targets/3/state',dict(enabled=True),method='PUT')
        self.wait_for(lambda s: len(s['incidents']) == 2)
        # Neither a conflicting port nor a conflicting database owner may reset a live outage.
        for port in [self.port,0]:
            result=subprocess.run([str(ROOT/'target/debug/netcheck'),'--port',str(port),'--database',self.db],capture_output=True,timeout=5)
            self.assertNotEqual(result.returncode,0)
            self.assertIn(b'already',result.stdout+result.stderr)
            self.assertEqual(self.request('/api/status')['incidents'][0]['outcome'],'ongoing')
        # SIGTERM closes incidents and stops probe tasks without needing a restart.
        self.process.terminate(); self.process.wait(timeout=10)
        with sqlite3.connect(self.db) as connection:
            self.assertEqual(connection.execute("SELECT COUNT(*) FROM incidents WHERE ended_at IS NULL").fetchone()[0],0)
        self.start_server()
        state = self.request('/api/status')
        self.assertEqual(len(state['targets']), 3)
        self.assertEqual(state['incidents'][0]['outcome'], 'interrupted')
        self.request('/api/targets/3', method='DELETE')
        self.assertEqual(len(self.request('/api/status')['targets']),2)
        csv = self.request('/api/incidents.csv').decode()
        self.assertIn("'=Unreachable",csv)
        self.assertIn('interrupted', csv)
        print('Verified concurrent user-supplied targets, thresholds, idempotent pause/resume, ownership, shutdown, restart, CSV and input/origin checks.')

if __name__ == '__main__': unittest.main()
