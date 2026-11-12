#!/usr/bin/env python3
"""Bootstrap explicit synthetic workforce fixtures and publish local product/terms."""
import json
import os
import re
import subprocess
import uuid
from identity import ROOT,BASE,admin_token,call
if os.environ.get('BANK_PROFILE','local')!='local':raise SystemExit('Synthetic seed requires the local profile')
database=os.environ.get('BANK_SEED_DATABASE','piggybank')
if database!='piggybank' and not re.fullmatch(r'piggybank_http_test_[0-9a-f]{32}',database):raise SystemExit('Seed database must be the local project or its disposable HTTP fixture')
def sql(statement):
    return subprocess.check_output(['podman','exec','piggybank-postgres-1','psql','-U','dev_admin','-d',database,'-At','-v','ON_ERROR_STOP=1','-c',statement],text=True).strip()
token=admin_token();issuer=BASE+'/realms/workforce';subjects={}
roles={'reviewer':['product_admin','case_reviewer','finance_adjuster','account_support','payment_inquiry','payment_operator'],'approver':['product_approver','case_approver','finance_approver','security_approver','payment_approver'],'security':['security_admin','operations_replay']}
for username,grants in roles.items():
    subject=str(uuid.UUID(call(token,'workforce/users?exact=true&username='+username)[0]['id']));subjects[username]=subject
    person=str(uuid.uuid5(uuid.NAMESPACE_URL,issuer+'/'+subject))
    sql(f"INSERT INTO controls.person_links VALUES('{issuer}','{subject}','{person}') ON CONFLICT DO NOTHING")
    person=str(uuid.UUID(sql(f"SELECT person_id FROM controls.person_links WHERE issuer='{issuer}' AND subject='{subject}'")))
    for role in grants:
        grant=str(uuid.uuid5(uuid.NAMESPACE_URL,'piggybank/synthetic-grant/'+person+'/'+role))
        sql(f"INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES('{grant}','{person}','{role}',now()+interval '365 days') ON CONFLICT(id) DO UPDATE SET revoked_at=NULL,valid_until=now()+interval '365 days'")
    if username in ['reviewer','approver']:
        capability='REVIEW' if username=='reviewer' else 'APPROVE'
        for queue in ['controls-operations','support','payments-operations','finance-operations','data-operations']:
            sql(f"INSERT INTO controls.case_queue_members(queue,person_id,capability) VALUES('{queue}','{person}','{capability}') ON CONFLICT(queue,person_id,capability) DO UPDATE SET revoked_at=NULL")
# A balanced funding command uses this zero-valued synthetic settlement ledger account.
sql("INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES('4f1a780b-3e31-48ef-a294-000000000003','SETTLEMENT','ACTIVE',false) ON CONFLICT(id) DO NOTHING")
subprocess.run([str(ROOT/'target/debug/bank-seed')],input=json.dumps({'issuer':issuer,'makerSubject':subjects['reviewer'],'checkerSubject':subjects['approver']}),text=True,cwd=ROOT,env=os.environ,check=True)
print('Synthetic workforce roles and explicit queue relationships configured. Bootstrap is local-only.')
