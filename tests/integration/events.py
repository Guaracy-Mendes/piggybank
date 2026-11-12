#!/usr/bin/env python3
"""Exercise persisted work and event projection across worker restart."""
import json
import subprocess
import time
import uuid
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
def sql(query):
    return subprocess.check_output(['podman','exec','piggybank-postgres-1','psql','-U','dev_admin','-d','piggybank','-At','-c',query],text=True).strip()
def wait(predicate,seconds=20):
    until=time.monotonic()+seconds
    while time.monotonic()<until:
        if predicate(): return
        time.sleep(.2)
    raise AssertionError('Durable worker condition timed out')
key='probe-'+uuid.uuid4().hex
operation=json.loads(subprocess.check_output([str(ROOT/'target/debug/bank-probe'),key],cwd=ROOT,text=True))['id']
worker=None
try:
    # Persist an abandoned lease. Restart must reclaim this original job, not enqueue a replacement.
    sql(f"UPDATE platform.jobs SET state='RUNNING',lease_owner=gen_random_uuid(),lease_until=now()-interval '1 second',fence=1,attempts=1 WHERE operation_id='{operation}'")
    for cycle in range(2):
        worker=subprocess.Popen([str(ROOT/'target/debug/bank-worker')],cwd=ROOT,stdout=subprocess.DEVNULL)
        wait(lambda:sql(f"SELECT state FROM platform.operations WHERE id='{operation}'")=='SUCCEEDED')
        wait(lambda:sql(f"SELECT state FROM data.operation_views WHERE operation_id='{operation}'")=='SUCCEEDED')
        worker.terminate();worker.wait(timeout=10);worker=None
    repeated=json.loads(subprocess.check_output([str(ROOT/'target/debug/bank-probe'),key],cwd=ROOT,text=True))
    assert repeated['id']==operation and repeated['state']=='SUCCEEDED'
    assert sql(f"SELECT count(*) FROM platform.jobs WHERE operation_id='{operation}'")=='1'
    assert sql(f"SELECT attempts FROM platform.jobs WHERE operation_id='{operation}'")=='2'
    assert sql(f"SELECT count(*) FROM platform.outbox WHERE aggregate_id='{operation}' AND delivered_at IS NOT NULL")=='3'
    assert sql(f"SELECT count(*) FROM data.inbox i JOIN platform.outbox o ON o.event_id=i.event_id WHERE o.aggregate_id='{operation}'")=='3'
    print('Worker restart: one original job, fenced recovery, three ordered facts and one durable projection passed.')
finally:
    if worker is not None:
        worker.terminate();worker.wait(timeout=10)
