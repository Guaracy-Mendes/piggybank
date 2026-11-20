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
offset=int(os.environ.get('BANK_BROWSER_PORT_OFFSET','0'))
if offset not in (0,200):raise SystemExit('Supported isolated browser offsets are 0 and 200.')
api,worker,provider,customer,staff=[x+offset for x in [4180,4181,4182,4300,4301]]
# Add only the isolated loopback callback; retain every existing issuer client setting.
if offset:
    sys.path.insert(0,str(ROOT/'scripts'))
    from identity import admin_token,call
    token=admin_token()
    for realm,trust,name in [('customers','customer','customer-web'),('workforce','workforce','staff-web')]:
        client=call(token,realm+'/clients?clientId='+name)[0]
        current=call(token,realm+'/clients/'+client['id'])
        uri=f'http://127.0.0.1:{api}/auth/{trust}/callback'
        if uri not in current.get('redirectUris',[]):
            current['redirectUris']=current.get('redirectUris',[])+[uri]
            call(token,realm+'/clients/'+client['id'],'PUT',current)
for port in [api,worker,provider,customer,staff]:
    with socket.socket() as probe:
        probe.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
        probe.bind(('127.0.0.1',port))
signal.signal(signal.SIGTERM,stop);signal.signal(signal.SIGINT,stop)
db=Database();processes=[]
manifest=ROOT/'.local/customer-browser-fixture.json'
try:
    # Functional suites share one loopback IP across multiple independent people.
    # Quota enforcement is tested separately by tests/integration/ingress.py.
    env=dict(db.env,BANK_PUBLIC_BASE=f'http://127.0.0.1:{api}',BANK_API_PROXY_TARGET=f'http://127.0.0.1:{api}',BANK_CUSTOMER_WEB_PORT=str(customer),BANK_WORKFORCE_WEB_PORT=str(staff),BANK_RAIL_PROVIDER_BASE=f'http://127.0.0.1:{provider}',BANK_SIMULATOR_URL=f'http://127.0.0.1:{provider}',BANK_FINANCE_PROVIDER_BASE=f'http://127.0.0.1:{provider}',BANK_CUSTOMER_WEB_ORIGIN=f'http://127.0.0.1:{customer}',BANK_WORKFORCE_WEB_ORIGIN=f'http://127.0.0.1:{staff}',BANK_RATE_LIMIT=os.environ.get('BANK_BROWSER_RATE_LIMIT','5000'))
    subprocess.run(['python3',str(ROOT/'scripts/seed.py')],cwd=ROOT,env=env,stdout=subprocess.DEVNULL,check=True)
    manifest.parent.mkdir(exist_ok=True)
    manifest.write_text(json.dumps({'database':db.name}));manifest.chmod(0o600)
    for name,port in [('bank-api',api),('bank-worker',worker),('provider-simulator',provider)]:
        # BANK_FIXTURE_TARGET=release runs optimised binaries (load qualification); functional suites use debug.
        command=[str(ROOT/'target'/os.environ.get('BANK_FIXTURE_TARGET','debug')/name)]
        # Optional sensitive-data inspection (SEC-05): capture service logs to an explicit directory.
        log_dir=os.environ.get('BANK_FIXTURE_LOG_DIR')
        sink=open(Path(log_dir)/f'{name}.log','w') if log_dir else subprocess.DEVNULL
        processes.append(subprocess.Popen(command,cwd=ROOT,env=dict(env,BANK_BIND=f'127.0.0.1:{port}'),start_new_session=True,stdout=sink,stderr=subprocess.STDOUT if log_dir else None))
    until=time.monotonic()+30
    while time.monotonic()<until:
        try:
            with urllib.request.urlopen(f'http://127.0.0.1:{api}/health/ready',timeout=2) as response:
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
