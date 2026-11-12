#!/usr/bin/env python3
"""Own the complete isolated local browser fixture and only its child process groups."""
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import time
import urllib.request
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'tests/integration'))
from database_fixture import Database
stopped=False
def stop(*_):
    global stopped
    stopped=True
for port in [4180,4181,4182,4300,4301]:
    with socket.socket() as probe:
        probe.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
        probe.bind(('127.0.0.1',port))
signal.signal(signal.SIGTERM,stop);signal.signal(signal.SIGINT,stop)
db=Database();processes=[]
manifest=ROOT/'.local/customer-browser-fixture.json'
try:
    # Functional suites share one loopback IP across multiple independent people.
    # Quota enforcement is tested separately by tests/integration/ingress.py.
    env=dict(db.env,BANK_CUSTOMER_WEB_ORIGIN='http://127.0.0.1:4300',BANK_WORKFORCE_WEB_ORIGIN='http://127.0.0.1:4301',BANK_RATE_LIMIT=os.environ.get('BANK_BROWSER_RATE_LIMIT','5000'))
    subprocess.run(['python3',str(ROOT/'scripts/seed.py')],cwd=ROOT,env=env,stdout=subprocess.DEVNULL,check=True)
    manifest.parent.mkdir(exist_ok=True)
    manifest.write_text(json.dumps({'database':db.name}));manifest.chmod(0o600)
    for command in [[str(ROOT/'target/debug'/name)] for name in ['bank-api','bank-worker','provider-simulator']]:
        processes.append(subprocess.Popen(command,cwd=ROOT,env=env,start_new_session=True,stdout=subprocess.DEVNULL))
    until=time.monotonic()+30
    while time.monotonic()<until:
        try:
            with urllib.request.urlopen('http://127.0.0.1:4180/health/ready',timeout=2) as response:
                if response.status==200:break
        except OSError:time.sleep(.2)
    else:raise RuntimeError('Bank API browser fixture readiness failed')
    processes.append(subprocess.Popen([str(ROOT/'scripts/npm.sh'),'run','dev','--workspace','@piggybank/customer-web'],cwd=ROOT,env=env,start_new_session=True,stdout=subprocess.DEVNULL))
    processes.append(subprocess.Popen([str(ROOT/'scripts/npm.sh'),'run','dev','--workspace','@piggybank/staff-web'],cwd=ROOT,env=env,start_new_session=True,stdout=subprocess.DEVNULL))
    while not stopped:
        if any(process.poll() is not None for process in processes):raise RuntimeError('A browser fixture process exited')
        time.sleep(.25)
finally:
    for process in processes:
        try:os.killpg(process.pid,signal.SIGTERM)
        except ProcessLookupError:pass
    for process in processes:
        try:process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid,signal.SIGKILL);process.wait()
    manifest.unlink(missing_ok=True)
    db.close()
