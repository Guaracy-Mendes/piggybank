#!/usr/bin/env python3
"""Prepare and own a persistent loopback-only Piggybank CX preview."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import time
import urllib.request

ROOT=Path(__file__).resolve().parents[1]
LOCAL=ROOT/'.local'
STATE=LOCAL/'cx-runtime.json'
PORTS=[4180,4181,4182,4300,4301]

def run(*args):
    subprocess.run(list(args),cwd=ROOT,check=True)

def identity(pid):
    try:
        proc=Path('/proc')/str(pid)
        command=(proc/'cmdline').read_bytes().split(b'\0')
        valid=str(ROOT/'scripts/dev.py').encode() in command and b'run' in command
        return (proc/'stat').read_text().rsplit(')',1)[1].split()[19] if valid else None
    except (FileNotFoundError,ProcessLookupError,PermissionError):return None

def owned():
    if not STATE.exists():return None
    state=json.loads(STATE.read_text())
    return state if identity(state['pid'])==state['startIdentity'] else None

def prepare():
    for key,val in os.environ.items():
        if key.endswith('_DATABASE_URL') and '127.0.0.1:15432/' not in val:
            raise SystemExit('CX preparation requires the local loopback database; use the deployment runbook for other environments.')
    run(sys.executable,str(ROOT/'scripts/database.py'))
    run(str(ROOT/'scripts/cargo.sh'),'build','--workspace','--locked')
    env=dict(os.environ,MIGRATION_DATABASE_URL='postgres://bank_migrator:synthetic-local-only@127.0.0.1:15432/piggybank')
    subprocess.run([str(ROOT/'target/debug/bank-migrate')],env=env,cwd=ROOT,check=True)
    run(sys.executable,str(ROOT/'scripts/seed.py'))
    print('Local schema and independently approved synthetic fixtures prepared. Existing money and immutable records retained.')

def start():
    if owned():print('CX preview is already running.');return
    for port in PORTS:
        with socket.socket() as probe:
            probe.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
            try:probe.bind(('127.0.0.1',port))
            except OSError:raise SystemExit(f'Port {port} is occupied. Its process was left untouched. Stop your existing fixture or preview first.')
    log=LOCAL/'cx-runtime.log'
    with log.open('ab') as stream:
        p=subprocess.Popen([sys.executable,str(ROOT/'scripts/dev.py'),'run'],cwd=ROOT,stdin=subprocess.DEVNULL,stdout=stream,stderr=subprocess.STDOUT,start_new_session=True)
    start_id=None
    for _ in range(50):
        start_id=identity(p.pid)
        if start_id:break
        if p.poll() is not None:raise SystemExit(f'Preview exited; see {log}')
        time.sleep(.02)
    if not start_id:raise SystemExit(f'Unable to establish preview ownership; inspect {log}')
    STATE.write_text(json.dumps({'pid':p.pid,'startIdentity':start_id,'log':str(log)},indent=2)+'\n');STATE.chmod(0o600)
    deadline=time.monotonic()+90
    while time.monotonic()<deadline:
        if not owned():raise SystemExit(f'Preview exited; see {log}')
        try:
            for url in ['http://127.0.0.1:4180/health/ready','http://127.0.0.1:4300','http://127.0.0.1:4301']:
                with urllib.request.urlopen(url,timeout=2) as response:
                    if response.status!=200:raise OSError('Not ready')
            print('Customer: http://127.0.0.1:4300\nStaff: http://127.0.0.1:4301\nStop only this preview: python3 scripts/local_cx.py stop')
            return
        except OSError:time.sleep(.3)
    raise SystemExit(f'Preview is still starting; see {log}. No unrelated process was stopped.')

def stop():
    state=owned()
    if state:
        os.kill(state['pid'],signal.SIGTERM)
        deadline=time.monotonic()+30
        while time.monotonic()<deadline and owned():time.sleep(.1)
        if owned():raise SystemExit('Preview is still shutting down; inspect its log. No forced termination performed.')
    STATE.unlink(missing_ok=True)
    print('Owned CX preview stopped. Dependency containers, database and other applications retained.')

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('action',choices=['prepare','start','stop','status']);args=p.parse_args();LOCAL.mkdir(exist_ok=True)
    with (LOCAL/'cx-runtime.lock').open('a') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX)
        if args.action=='prepare':prepare()
        elif args.action=='start':start()
        elif args.action=='stop':stop()
        else:print('CX preview is running.' if owned() else 'No owned CX preview is running.')
if __name__=='__main__':main()
