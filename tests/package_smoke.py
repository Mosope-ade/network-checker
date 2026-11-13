"""Check a built executable's real startup, ping, storage ownership and exports."""
import json
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request


def main():
    executable = str(Path(sys.argv[1]).resolve())
    with tempfile.TemporaryDirectory(prefix='netcheck-package-') as directory:
        root = Path(directory)
        database = root / 'data with spaces' / 'netcheck.db'
        with socket.socket() as reservation:
            reservation.bind(('localhost', 0))
            port = reservation.getsockname()[1]
        base = f'http://localhost:{port}'

        def request(path, data=None, method=None):
            req = urllib.request.Request(base+path, data=json.dumps(data).encode() if data is not None else None,
                                         headers={'Content-Type':'application/json'},method=method)
            with urllib.request.urlopen(req, timeout=5) as response:
                content=response.read()
                return json.loads(content) if 'application/json' in response.headers.get('Content-Type','') else content

        with (root/'server.log').open('w+', encoding='utf-8') as log:
            process = subprocess.Popen([executable,'--port',str(port),'--database',str(database)],stdout=log,stderr=subprocess.STDOUT)
            try:
                until=time.monotonic()+20
                while True:
                    if process.poll() is not None: raise AssertionError('Packaged executable exited during startup')
                    try:
                        state=request('/api/status')
                        break
                    except (OSError,urllib.error.URLError):
                        if time.monotonic()>until: raise AssertionError('Packaged executable did not start in time')
                        time.sleep(.2)
                assert state['targets']==[], 'Package must not ship preset targets'
                assert b'Network monitor' in request('/'), 'Embedded GUI was not served'
                # The test operator selects the platform loopback host; no address is embedded.
                request('/api/targets',dict(name='Package verification',address=socket.gethostbyname('localhost'),interval=1,failure_threshold=2,recovery_threshold=2))
                until=time.monotonic()+15
                while True:
                    target=request('/api/status')['targets'][0]
                    if target['status']=='up': break
                    if target['status']=='error': raise AssertionError(target['error'])
                    if time.monotonic()>until: raise AssertionError(f"Loopback did not respond: {target['status']} {target['error']}")
                    time.sleep(.2)
                conflict=subprocess.run([executable,'--port','0','--database',str(database)],capture_output=True,text=True,timeout=5)
                assert conflict.returncode!=0, 'A second monitor acquired the database'
                request('/api/targets/1/state',dict(enabled=False),method='PUT')
                assert request('/api/status')['targets'][0]['status']=='paused'
                assert b'timestamp_utc' in request('/api/activity.csv')
                assert b'target,address' in request('/api/incidents.csv')
                print('Release executable passed: embedded GUI, no preset targets, actual ICMP reply, SQLite writes, exclusive ownership, pause and streamed exports.')
            except BaseException:
                log.flush(); log.seek(0); print(log.read(),file=sys.stderr)
                raise
            finally:
                process.terminate()
                process.wait(timeout=10)


if __name__=='__main__': main()
