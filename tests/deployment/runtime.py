#!/usr/bin/env python3
"""Run the cloud image set in a disposable shared-network pod; retain local dev services."""
import argparse
import base64
import hashlib
import hmac
from datetime import datetime, timezone, timedelta
import urllib.error
import urllib.request
import importlib.util
import json
import os
import re
from pathlib import Path
import subprocess
import tempfile
import uuid
from artifacts import extract_release

ROOT=Path(__file__).resolve().parents[2]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('release')
args=parser.parse_args()
name='piggybank-cloud-test-'+uuid.uuid4().hex[:10]
images=json.loads((ROOT/'infra/versions.json').read_text())['images']

def run(*args, **kw):
    try:
        return subprocess.run(['podman',*args],check=True,**kw)
    except subprocess.CalledProcessError as error:
        # Disposable credentials can appear in container environment arguments.
        raise RuntimeError(f'Cloud acceptance container command failed with status {error.returncode}; arguments withheld') from None

def verify_clearing_provider(key, schema):
    """Exercise packaged provider namespaces and signatures using this installation's random key."""
    now=datetime.now(timezone.utc)
    file={'fileId':str(uuid.uuid4()),'formatVersion':1,'provider':'SYNTHETIC_RAIL_SETTLEMENT',
          'feePolicy':'SYNTHETIC_PROVIDER_ZERO_FEE_V1',
          'range':{'fromDate':now.date().isoformat(),'toDate':(now+timedelta(days=1)).date().isoformat()},
          'generatedAt':now.isoformat(),'rows':[]}
    scopes=[]
    if schema>=45:scopes.append(('settlement-files','SYNTHETIC_OUTGOING_RECONCILIATION_V1'))
    if schema>=46:scopes.append(('credit-settlement-files','SYNTHETIC_CREDIT_RECONCILIATION_V1'))
    def canonical(value):return json.dumps(value,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()
    def signature(timestamp,body):
        digest=hashlib.sha256(canonical(body)).hexdigest()
        return base64.urlsafe_b64encode(hmac.new(key.encode(),f'{timestamp}\n{digest}'.encode(),hashlib.sha256).digest()).decode().rstrip('=')
    def post(path,body,headers=None):
        request=urllib.request.Request('http://127.0.0.1:24182'+path,data=canonical(body),headers={'Content-Type':'application/json',**(headers or {})},method='POST')
        class NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self,*args,**kwargs):return None
        with urllib.request.build_opener(NoRedirect()).open(request,timeout=5) as response:return response.status,response.read()
    group_count=0
    for endpoint,kind in scopes:
        assert post('/admin/'+endpoint,file,{'X-Finance-Key':key})[0]==201
        assert post('/admin/'+endpoint,file,{'X-Finance-Key':key})[0]==200
        query={'kind':kind,'fileId':file['fileId'],'requestId':str(uuid.uuid4())}
        stamp=int(datetime.now(timezone.utc).timestamp())
        status,body=post('/'+endpoint+'/inquiry',{'timestamp':stamp,'body':query,'signature':signature(stamp,query)})
        assert status==200
        packet=json.loads(body)
        assert abs(int(datetime.now(timezone.utc).timestamp())-packet['timestamp'])<=60
        assert hmac.compare_digest(packet['signature'],signature(packet['timestamp'],packet['body']))
        assert packet['body']['request']==query and packet['body']['file']['fileId']==file['fileId']
        assert packet['body']['file']['range']==file['range'] and packet['body']['file']['rows']==[]
        if schema>=47:
            typed_file=packet['body']['file']
            second={**typed_file,'fileId':str(uuid.uuid4())}
            assert post('/admin/'+endpoint,second,{'X-Finance-Key':key})[0]==201
            publish={'setId':str(uuid.uuid4()),'kind':'OUTGOING' if endpoint=='settlement-files' else 'CREDITS',
                     'range':typed_file['range'],'generatedAt':typed_file['generatedAt'],'fileIds':[second['fileId'],typed_file['fileId']]}
            status,body=post('/admin/settlement-sets',publish,{'X-Finance-Key':key});assert status==201
            sealed=json.loads(body);assert sealed['rowCount']==0 and len(sealed['members'])==2
            expected={f['fileId']:hashlib.sha256(canonical(f)).hexdigest() for f in [typed_file,second]}
            assert [m['fileId'] for m in sealed['members']]==sorted(expected)
            assert all(m['digest']==expected[m['fileId']] and m['rowCount']==0 for m in sealed['members'])
            assert post('/admin/settlement-sets',publish,{'X-Finance-Key':key})[0]==200
            query={'kind':'SYNTHETIC_SETTLEMENT_SET_RECONCILIATION_V1','setId':publish['setId'],'requestId':str(uuid.uuid4())}
            stamp=int(datetime.now(timezone.utc).timestamp())
            status,body=post('/settlement-sets/inquiry',{'timestamp':stamp,'body':query,'signature':signature(stamp,query)});assert status==200
            signed=json.loads(body)
            assert abs(int(datetime.now(timezone.utc).timestamp())-signed['timestamp'])<=60
            assert hmac.compare_digest(signed['signature'],signature(signed['timestamp'],signed['body']))
            assert signed['body']['request']==query and signed['body']['manifest']==sealed
            group_count+=1
    if schema>=46:
        paths=['credit-reconciliations']+(['reconciliation-groups'] if schema>=47 else [])+(['periodic-policies'] if schema>=48 else [])+(['corrections'] if schema>=50 else [])+(['periodic-bases'] if schema>=51 else [])
        for port,expected,path in [(port,expected,path) for path in paths for port,expected in [(24300,403),(24301,401)]]:
            try:
                with urllib.request.urlopen(f'http://127.0.0.1:{port}/api/v1/staff/finance/{path}',timeout=5) as response:status=response.status
            except urllib.error.HTTPError as error:
                status=error.code;error.close()
            assert status==expected
    print(f'Packaged clearing publication/replay and authenticated inquiry passed for {len(scopes)} file namespaces and {group_count} closed signed groups; channel guards passed.',flush=True)

def verify_periodic_portal(schema):
    """Inspect the compiled portal served by the actual archived web image."""
    if schema<49:return
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self,*args,**kwargs):return None
    opener=urllib.request.build_opener(NoRedirect())
    with opener.open('http://127.0.0.1:24301/',timeout=5) as response:page=response.read(65537)
    assert len(page)<=65536
    script=re.search(rb'<script\b[^>]*\bsrc="([^"]+)"',page)
    assert script,'Compiled workforce script missing'
    path=script.group(1).decode('ascii')
    assert path.startswith('/assets/') and '..' not in path and '?' not in path
    with opener.open('http://127.0.0.1:24301'+path,timeout=5) as response:bundle=response.read(1048577)
    assert len(bundle)<=1048576
    assert all(label in bundle for label in [b'Periodic policies',b'Approve preview policy',b'Due at (UTC)',b'Evaluated at (UTC)'])
    print('Actual archived workforce bundle contains periodic policy review and explicit UTC preview clocks.',flush=True)
    if schema>=50:
        assert all(label in bundle for label in [b'Financial corrections',b'Execute approved correction',b'Native correction receipt',b'Recovered the original native correction receipt'])
        print('Actual archived workforce bundle contains controlled financial correction review and receipts.',flush=True)
    if schema>=51:
        assert all(label in bundle for label in [b'Native ledger basis',b'Capture fresh observation',b'Native source facts',b'OPEN_PERIOD_PROVISIONAL',b'Current booked at capture',b'Observed closing booked'])
        print('Actual archived workforce bundle contains native basis capture, provisional status and distinct historical/current positions.',flush=True)

with tempfile.TemporaryDirectory(prefix='cloud-runtime-',dir=ROOT/'.local') as directory:
    directory=Path(directory)
    archive=ROOT/'.local/cloud/releases'/f'{args.release}.tgz'
    manifest=extract_release(archive,directory,args.release)
    deployment=directory/'deployment'
    run('load','-i',str(directory/'images.tar'),stdout=subprocess.DEVNULL)
    print(f"Verified release archive and loaded its images: {args.release}, schema {manifest['schemaVersion']}",flush=True)
    os.environ['BANK_CLOUD_STATE']=str(directory/'state')
    os.environ['BANK_IDENTITY_ADMIN_BASE']='http://127.0.0.1:38280'
    spec=importlib.util.spec_from_file_location('cloud_bootstrap',deployment/'bootstrap.py')
    bootstrap=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(bootstrap)
    bootstrap.create_environment()
    control=bootstrap.load_env(directory/'state/.env')
    app=bootstrap.load_env(directory/'state/runtime.env')
    try:
        ports=[]
        for port in [4180,4181,4182,4300,4301,18280,18222]:
            ports+=['-p',f'127.0.0.1:{port+20000}:{port+10000}']
        run('pod','create','--name',name,*ports,stdout=subprocess.DEVNULL)
        # The app stays on pod loopback. A test-only HTTP bridge represents the
        # provider tunnels, since published container ports target the pod NIC.
        proxy=directory/'test-proxy.conf'
        proxy.write_text('events {}\nhttp {\n'+''.join(
            f'server {{ listen {port+10000}; location / {{ proxy_pass http://127.0.0.1:{port}; }} }}\n'
            for port in [4180,4181,4182,4300,4301,18280,18222])+'}\n')
        run('run','-d','--pod',name,'--name',name+'-bridge','-v',str(proxy)+':/test-proxy.conf:ro',
            f'piggybank-web:{args.release}','nginx','-c','/test-proxy.conf','-g','daemon off;',stdout=subprocess.DEVNULL)
        run('volume','create',name+'-pg',stdout=subprocess.DEVNULL)
        run('volume','create',name+'-evidence',stdout=subprocess.DEVNULL)
        run('volume','create',name+'-nats',stdout=subprocess.DEVNULL)
        run('run','-d','--pod',name,'--name',name+'-pg',
            '-e','POSTGRES_USER=bank_admin','-e','POSTGRES_DB=piggybank',
            '-e','POSTGRES_PASSWORD='+control['POSTGRES_PASSWORD'],
            '-v',name+'-pg:/var/lib/postgresql',images['postgres'],
            'postgres','-p','15432','-c','listen_addresses=127.0.0.1','-c','max_connections=200',stdout=subprocess.DEVNULL)
        import time
        for _ in range(60):
            p=subprocess.run(['podman','exec',name+'-pg','pg_isready','-p','15432','-U','bank_admin'],capture_output=True)
            if p.returncode==0:break
            time.sleep(1)
        else:raise RuntimeError('PG did not start')
        def compose(*command,data=None,capture=False):
            if command[:3]==('exec','-T','postgres'):
                return run('exec','-i',name+'-pg',*command[3:],input=data,text=True,capture_output=capture)
            if command==('run','--rm','-T','seed'):
                return run('run','--rm','-i','--pod',name,'--env-file',str(directory/'state/runtime.env'),f'piggybank-runtime:{args.release}','bank-seed',input=data,text=True,capture_output=capture)
            raise AssertionError(command)
        bootstrap.compose=compose
        bootstrap.database()
        run('run','--rm','--pod',name,'-e','MIGRATION_DATABASE_URL='+control['MIGRATION_DATABASE_URL'],f'piggybank-runtime:{args.release}','bank-migrate')
        run('run','-d','--pod',name,'--name',name+'-nats','-v',name+'-nats:/data',
            '-v',str(deployment/'nats.conf')+':/etc/nats/cloud.conf:ro',images['nats'],'-c','/etc/nats/cloud.conf',stdout=subprocess.DEVNULL)
        run('run','-d','--pod',name,'--name',name+'-keycloak',
            '-e','KC_DB=postgres','-e','KC_DB_URL=jdbc:postgresql://127.0.0.1:15432/keycloak',
            '-e','KC_DB_USERNAME=keycloak','-e','KC_DB_PASSWORD='+control['KEYCLOAK_DB_PASSWORD'],
            '-e','KC_HOSTNAME=http://127.0.0.1:18280','-e','KC_HEALTH_ENABLED=true',
            '-e','KC_BOOTSTRAP_ADMIN_USERNAME=local-admin',
            '-e','KC_BOOTSTRAP_ADMIN_PASSWORD='+control['KEYCLOAK_ADMIN_PASSWORD'],
            '-v',str(deployment/'identity')+':/opt/keycloak/data/import:ro',images['keycloak'],
            'start-dev','--import-realm','--http-host=127.0.0.1','--http-port=18280','--http-management-host=127.0.0.1',stdout=subprocess.DEVNULL)
        subprocess.run(['python3',str(deployment/'wait.py'),'http://127.0.0.1:38280/realms/customers/.well-known/openid-configuration'],check=True)
        bootstrap.seed()
        for service,binary in [('provider','provider-simulator'),('api','bank-api'),('worker','bank-worker')]:
            run('run','-d','--pod',name,'--name',name+'-'+service,'--env-file',str(directory/'state/runtime.env'),
                '-v',name+'-evidence:/evidence:U',f'piggybank-runtime:{args.release}',binary,stdout=subprocess.DEVNULL)
        run('run','-d','--pod',name,'--name',name+'-web',f'piggybank-web:{args.release}',stdout=subprocess.DEVNULL)
        for port in [24180,24181]:
            subprocess.run(['python3',str(deployment/'wait.py'),f'http://127.0.0.1:{port}/health/ready'],check=True)
        subprocess.run(['python3',str(deployment/'smoke.py')],env={**os.environ,'BANK_CLOUD_SMOKE_PORT_OFFSET':'20000'},check=True)
        verify_clearing_provider(app['BANK_FINANCE_SIGNING_KEY'],manifest['schemaVersion'])
        verify_periodic_portal(manifest['schemaVersion'])
        if manifest['schemaVersion']>=50:
            usage=subprocess.run(['podman','run','--rm','--pod',name,'--env-file',str(directory/'state/runtime.env'),f'piggybank-runtime:{args.release}','bank-fund'],capture_output=True,text=True)
            assert usage.returncode==1 and 'Usage: bank-fund' in usage.stderr
            print('Packaged native funding fixture command is available; no financial posting was made.',flush=True)
        # Repeat immutable bootstrap commands without regranting revoked authorities.
        bootstrap.seed()
        print('Cloud image migration, randomized identity setup, approved fixture publication and repeated seed passed.')
    except BaseException:
        for service in ['pg','keycloak','nats','provider','api','worker','web']:
            subprocess.run(['podman','logs','--tail','40',name+'-'+service],check=False)
        raise
    finally:
        subprocess.run(['podman','pod','rm','--force',name],check=False,stdout=subprocess.DEVNULL)
        for suffix in ['pg','nats','evidence']:
            subprocess.run(['podman','volume','rm','--force',name+'-'+suffix],check=False,stdout=subprocess.DEVNULL)
