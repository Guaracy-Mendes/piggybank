#!/usr/bin/env python3
"""Real OIDC HTTP lifecycle with worker/provider processes and isolated durable stores."""
import json
import subprocess
import time
import uuid
import urllib.request
from database_fixture import Database,ROOT
from identity import browser_login,call,BASE

def wait(check,seconds=35):
    until=time.monotonic()+seconds
    while time.monotonic()<until:
        try:
            value=check()
            if value:return value
        except (OSError,AssertionError):pass
        time.sleep(.2)
    raise AssertionError('Lifecycle condition timed out')
def start(name,env):return subprocess.Popen([str(ROOT/'target/debug'/name)],cwd=ROOT,env=env,stdout=subprocess.DEVNULL)
def main():
    db=Database();processes=[]
    try:
        subprocess.run(['python3',str(ROOT/'scripts/seed.py')],cwd=ROOT,env=db.env,check=True,stdout=subprocess.DEVNULL)
        subprocess.run(['python3',str(ROOT/'scripts/seed.py')],cwd=ROOT,env=db.env,check=True,stdout=subprocess.DEVNULL)
        provider=start('provider-simulator',db.env);worker=start('bank-worker',db.env);api=start('bank-api',db.env);processes.extend([provider,worker,api])
        wait(lambda:call(urllib.request.build_opener(),'/health/ready')[0]==200)
        alice,alice_jar,_=browser_login('customer','alice');bob,bob_jar,_=browser_login('customer','bob')
        def send(opener,jar,path,method='GET',body=None,key=None,expected=200):
            headers={}
            if method!='GET':
                headers={'Origin':BASE,'X-CSRF-Token':next(c.value for c in jar if c.name=='pb_csrf_customer')}
            if body is not None:headers['Content-Type']='application/json'
            if key:headers['Idempotency-Key']=key
            status,raw,_=call(opener,path,method,json.dumps(body).encode() if body is not None else None,headers)
            assert status==expected,(path,method,status,expected)
            return json.loads(raw) if raw else None
        product=send(alice,alice_jar,'/api/v1/products')[0]
        terms=send(alice,alice_jar,'/api/v1/products/'+product['id']+'/terms')
        profile={'fullName':'Synthetic Shared Applicant','email':'shared@example.invalid','dateOfBirth':'1990-01-01','residency':'SYNTHETIC','consentVersion':'synthetic-consent-v1','marketing':True}
        def complete(opener,jar,other,other_jar):
            key=str(uuid.uuid4());body={'productVersionId':product['id'],'marketing':True,'attribution':'integration_fixture'}
            app=send(opener,jar,'/api/v1/applications','POST',body,key,201)
            assert send(opener,jar,'/api/v1/applications','POST',body,key,201)['id']==app['id']
            send(other,other_jar,'/api/v1/applications/'+app['id'],expected=404)
            path='/api/v1/applications/'+app['id']
            assert call(opener,path+'/profile','PUT',json.dumps({'version':app['version'],'profile':profile}).encode(),{'Content-Type':'application/json','Origin':BASE})[0]==403
            app=send(opener,jar,path+'/profile','PUT',{'version':app['version'],'profile':profile})
            app=send(opener,jar,path+'/evidence','POST',{'content':'Synthetic identity evidence for local integration.'})
            app=send(opener,jar,path+'/terms','POST',{'termsId':terms['id']})
            request=str(uuid.uuid4());operation=send(opener,jar,path+'/submit','POST',key=request,expected=202)
            assert send(opener,jar,path+'/submit','POST',key=request,expected=202)['id']==operation['id']
            def active():
                current=send(opener,jar,path)
                return current if current['state']=='ACTIVE' else None
            current=wait(active)
            wait(lambda:send(opener,jar,'/api/v1/operations/'+operation['id'])['state']=='SUCCEEDED')
            return current,operation
        first,first_operation=complete(alice,alice_jar,bob,bob_jar)
        second,_=complete(bob,bob_jar,alice,alice_jar)
        assert first['accountId']!=second['accountId']
        assert db.sql('SELECT count(*) FROM core.parties')=='2'
        assert db.sql('SELECT count(*) FROM core.account_registry')=='2'
        assert db.sql('SELECT count(*) FROM core.journal_batches')=='0'
        assert db.sql('SELECT sum(booked)::bigint FROM core.ledger_accounts')=='0'
        send(alice,alice_jar,'/api/v1/applications/'+first['id']+'/marketing-withdrawal','POST',expected=204)
        assert db.sql(f"SELECT marketing::text||':'||(attribution IS NULL)::text FROM journeys.applications WHERE id='{first['id']}'")=='false:true'
        # Inquiry is always authorized against Core; financial fixture funding is balanced and idempotent.
        account=first['accountId'];account_path='/api/v1/accounts/'+account
        assert send(alice,alice_jar,account_path+'/balance')['available']['minorUnits']=='0'
        send(bob,bob_jar,account_path+'/balance',expected=404)
        def fund(value,operation=None):
            operation=operation or str(uuid.uuid4())
            subprocess.run([str(ROOT/'target/debug/bank-fund'),operation,account,str(value)],cwd=ROOT,env=db.env,check=True,stdout=subprocess.DEVNULL)
            return operation
        funding=fund(1000);fund(1000,funding);fund(2000)
        from datetime import datetime,timezone,timedelta
        day=datetime.now(timezone.utc).date()
        query=f"?from={day-timedelta(days=1)}&to={day+timedelta(days=1)}&limit=1"
        page=send(alice,alice_jar,account_path+'/transactions'+query);assert page['total']==2 and len(page['items'])==1
        fund(3000)
        next_page=send(alice,alice_jar,account_path+'/transactions'+query+f"&snapshotId={page['next']['snapshotId']}&after={page['next']['after']}")
        assert next_page['total']==2 and next_page['next'] is None and next_page['items'][0]['id']!=page['items'][0]['id']
        assert send(alice,alice_jar,account_path+'/transactions'+query)['total']==3
        send(bob,bob_jar,account_path+'/transactions'+query,expected=404)
        balance=send(alice,alice_jar,account_path+'/balance');assert balance['booked']['minorUnits']=='6000' and balance['held']['minorUnits']=='0' and balance['source']=='CORE_AUTHORITATIVE'
        send(alice,alice_jar,account_path+'/closure','POST',expected=409)
        send(bob,bob_jar,account_path+'/closure','POST',expected=404)
        request=str(uuid.uuid4());case=send(alice,alice_jar,account_path+'/service-requests','POST',key=request,expected=201)
        assert send(alice,alice_jar,account_path+'/service-requests','POST',key=request,expected=201)==case
        send(alice,alice_jar,account_path+'/freeze','POST',expected=204)
        assert next(a for a in send(alice,alice_jar,'/api/v1/accounts') if a['id']==account)['debitBlocked']
        send(bob,bob_jar,'/api/v1/accounts/'+second['accountId']+'/closure','POST',expected=204)
        send(bob,bob_jar,'/api/v1/accounts/'+second['accountId']+'/closure','POST',expected=204)
        change=send(alice,alice_jar,'/api/v1/profile/contact-changes','POST',{'email':'verified-contact@example.invalid'},str(uuid.uuid4()),202)
        send(bob,bob_jar,'/api/v1/profile/contact-changes/'+change['id'],expected=404)
        wait(lambda:send(alice,alice_jar,'/api/v1/profile/contact-changes/'+change['id'])['state']=='VERIFIED')
        profile=send(alice,alice_jar,'/api/v1/profile');assert profile['email']=='verified-contact@example.invalid' and profile['verifiedAt']
        wait(lambda:db.sql("SELECT count(*) FROM data.account_views WHERE state='CLOSED'")=='1')
        wait(lambda:db.sql('SELECT count(*) FROM data.transaction_views')=='6')
        wait(lambda:db.sql("SELECT source_version FROM data.master_refs WHERE id='"+profile['partyId']+"'")=='2')
        assert db.sql('SELECT sum(booked)::bigint FROM core.ledger_accounts')=='0'
        csrf=next(c.value for c in alice_jar if c.name=='pb_csrf_customer')
        assert call(alice,'/auth/customer/logout','POST',b'',{'X-CSRF-Token':csrf,'Origin':BASE})[0]==200
        alice,alice_jar,_=browser_login('customer','alice')
        # Recheck/recovery steps are valid additional jobs. Preserve exact journey
        # job identities across restart, independently of Finance export discovery.
        job_scope=" WHERE kind IN ('journey.onboard','profile.contact')"
        job_query="SELECT jsonb_agg(jsonb_build_array(id,operation_id,kind,step_key) ORDER BY id) FROM platform.jobs"+job_scope
        original_jobs=db.sql(job_query)
        for process in [api,worker]:process.terminate();process.wait(timeout=10);processes.remove(process)
        worker=start('bank-worker',db.env);api=start('bank-api',db.env);processes.extend([worker,api])
        wait(lambda:call(urllib.request.build_opener(),'/health/ready')[0]==200)
        resumed=send(alice,alice_jar,'/api/v1/applications/'+first['id'])
        assert resumed['accountId']==first['accountId'] and resumed['operationId']==first_operation['id'] and resumed['state']=='ACTIVE'
        assert send(alice,alice_jar,'/api/v1/operations/'+first_operation['id'])['state']=='SUCCEEDED'
        assert db.sql('SELECT count(*) FROM core.account_registry')=='2'
        assert db.sql("SELECT count(*) FROM platform.operations"+job_scope)=='3'
        assert db.sql("SELECT count(*) FROM platform.jobs"+job_scope+" AND step_key='initial'")=='3'
        assert db.sql(job_query)==original_jobs
        # The three real native funding journals are independently exported once.
        wait(lambda:db.sql("SELECT count(*) FROM finance.gl_exports WHERE state='BOOKED'")=='3')
        assert db.sql('SELECT count(DISTINCT source_batch_id) FROM finance.gl_exports')=='3'
        print('Real OIDC lifecycle passed: onboarding/restart, scoped balances, stable history, balanced funding, owned support, freeze/closure, verified contacts, stable original journey jobs, three unique booked GL exports and durable Core/Data projections.')
    finally:
        for process in processes:process.terminate()
        for process in processes:
            try:process.wait(timeout=10)
            except subprocess.TimeoutExpired:process.kill();process.wait()
        db.close()
if __name__=='__main__':main()
