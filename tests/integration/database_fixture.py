"""Isolate HTTP suites from persistent developer identities and financial records."""
import os
import shutil
import subprocess
import uuid
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
class Database:
    def __init__(self):
        self.name='piggybank_http_test_'+uuid.uuid4().hex
        self.admin('postgres',f'CREATE DATABASE {self.name} OWNER bank_migrator')
        self.env=dict(os.environ)
        suffix=self.name.removeprefix('piggybank_http_test_')
        self.object_root=ROOT/'.local'/('http_test_'+suffix)
        self.env['BANK_EVIDENCE_DIR']=str(self.object_root/'evidence')
        self.env['BANK_EVENT_NAMESPACE']='bank.http_test_'+suffix
        self.env['BANK_EVENT_STREAM']='BANK_HTTP_TEST_'+suffix
        self.env['BANK_SEED_DATABASE']=self.name
        for module in ['platform','identity','core','controls','journeys','payments','finance','documents','data','simulator']:
            url=f'postgres://bank_{module}:synthetic-{module}-only@127.0.0.1:15432/{self.name}'
            self.env['BANK_'+module.upper()+'_DATABASE_URL']=url
            if module=='platform':self.env['DATABASE_URL']=url
        self.env['MIGRATION_DATABASE_URL']=f'postgres://bank_migrator:synthetic-local-only@127.0.0.1:15432/{self.name}'
        try:subprocess.run([str(ROOT/'target/debug/bank-migrate')],env=self.env,cwd=ROOT,check=True,stdout=subprocess.DEVNULL)
        except BaseException:self.close();raise
    @staticmethod
    def admin(database,statement):
        return subprocess.check_output(['podman','exec','piggybank-postgres-1','psql','-U','dev_admin','-d',database,'-At','-v','ON_ERROR_STOP=1','-c',statement],text=True).strip()
    def sql(self,statement):return self.admin(self.name,statement)
    def close(self):
        cleanup=ROOT/'target/debug/bank-fixture-cleanup'
        if cleanup.exists():subprocess.run([str(cleanup),self.name.removeprefix('piggybank_http_test_')],env=self.env,cwd=ROOT,check=True,stdout=subprocess.DEVNULL)
        self.admin('postgres',f'DROP DATABASE IF EXISTS {self.name} WITH (FORCE)')
        if self.object_root.exists():shutil.rmtree(self.object_root)
