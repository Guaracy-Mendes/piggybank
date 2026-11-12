#!/usr/bin/env python3
"""First-boot identities and DB roles. Never prints credentials or tokens."""
import importlib.util
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import time
import urllib.request
import uuid

STATE = Path(os.environ.get('BANK_CLOUD_STATE', '/srv/piggybank/state'))
HERE = Path(__file__).resolve().parent
MODULES = ['platform', 'identity', 'core', 'controls', 'journeys', 'payments', 'finance', 'documents', 'data', 'simulator']

def load_env(path):
    return dict(line.split('=', 1) for line in path.read_text().splitlines() if line and not line.startswith('#'))

def create_environment():
    STATE.mkdir(mode=0o700, parents=True, exist_ok=True)
    control = STATE / '.env'
    runtime = STATE / 'runtime.env'
    if control.exists() != runtime.exists():
        raise SystemExit('Incomplete credential set: restore both files together; do not regenerate against existing data')
    if control.exists():
        return
    # Refuse to create new keys/passwords over an existing, restored data directory.
    if (STATE / 'postgres').exists() and any((STATE / 'postgres').iterdir()):
        raise SystemExit('Existing PostgreSQL without credentials: restore the original state backup')
    secret = lambda: secrets.token_hex(32)
    admin = {name: secret() for name in ['POSTGRES_PASSWORD', 'MIGRATOR_PASSWORD', 'KEYCLOAK_DB_PASSWORD',
                                        'KEYCLOAK_ADMIN_PASSWORD', 'BANK_FIXTURE_PASSWORD']}
    admin['MIGRATION_DATABASE_URL'] = f"postgres://bank_migrator:{admin['MIGRATOR_PASSWORD']}@127.0.0.1:15432/piggybank"
    app = {'BANK_PROFILE': 'local', 'BANK_EVIDENCE_DIR': '/evidence',
           'BANK_CUSTOMER_WEB_ORIGIN': 'http://127.0.0.1:4300',
           'BANK_WORKFORCE_WEB_ORIGIN': 'http://127.0.0.1:4301',
           'BANK_PUBLIC_BASE': 'http://127.0.0.1:4180',
           'BANK_ISSUER_BASE': 'http://127.0.0.1:18280',
           'OIDC_ISSUER': 'http://127.0.0.1:18280/realms/customers',
           'NATS_URL': 'nats://127.0.0.1:14222', 'RUST_LOG': 'info'}
    for module in MODULES:
        app[f'BANK_{module.upper()}_DATABASE_URL'] = f'postgres://bank_{module}:{secret()}@127.0.0.1:15432/piggybank'
    app['DATABASE_URL'] = app['BANK_PLATFORM_DATABASE_URL']
    for name in ['BANK_CUSTOMER_CLIENT_SECRET', 'BANK_WORKFORCE_CLIENT_SECRET', 'BANK_SERVICE_CLIENT_SECRET',
                 'BANK_SIMULATOR_KEY', 'BANK_RAIL_SIGNING_KEY', 'BANK_FINANCE_SIGNING_KEY', 'BANK_LIQUIDITY_SIGNING_KEY']:
        app[name] = secret()
    for path, values in [(control, admin), (runtime, app)]:
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, 'w') as stream:
            stream.write(''.join(f'{key}={value}\n' for key, value in values.items()))
    print('Generated persistent per-install credentials; values withheld.')

def compose(*args, data=None, capture=False):
    command = ['docker', 'compose', '--env-file', str(STATE / '.env'), '--env-file', str(HERE / 'release.env'),
               '-f', str(HERE / 'compose.yaml'), *args]
    return subprocess.run(command, input=data, text=True, check=True, capture_output=capture)

def sql(statement):
    return compose('exec', '-T', 'postgres', 'psql', '-U', 'bank_admin', '-p', '15432', '-d', 'piggybank',
                   '-At', '-v', 'ON_ERROR_STOP=1', data=statement, capture=True).stdout.strip()

def database():
    from urllib.parse import urlsplit
    admin = load_env(STATE / '.env')
    app = load_env(STATE / 'runtime.env')
    roles = {'bank_migrator': admin['MIGRATOR_PASSWORD'], 'keycloak': admin['KEYCLOAK_DB_PASSWORD']}
    roles.update({f'bank_{module}': urlsplit(app[f'BANK_{module.upper()}_DATABASE_URL']).password for module in MODULES})
    for name, password in roles.items():
        if not password or any(c not in '0123456789abcdef' for c in password):
            raise SystemExit('Expected generated hexadecimal DB credential')
        sql(f"DO $$ BEGIN IF NOT EXISTS(SELECT FROM pg_roles WHERE rolname='{name}') THEN CREATE ROLE {name} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT PASSWORD '{password}'; END IF; END $$;")
    sql('ALTER DATABASE piggybank OWNER TO bank_migrator; REVOKE CREATE ON SCHEMA public FROM PUBLIC;')
    if sql("SELECT count(*) FROM pg_database WHERE datname='keycloak'") == '0':
        sql('CREATE DATABASE keycloak OWNER keycloak;')
    print('Separate migration, identity and ten module principals ready.')

def seed():
    os.environ.update(load_env(STATE / '.env'))
    os.environ.update(load_env(STATE / 'runtime.env'))
    os.environ['BANK_REALM_DIR'] = str(HERE / 'identity')
    spec = importlib.util.spec_from_file_location('identity_provision', HERE / 'identity_provision.py')
    identity = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(identity)
    identity.provision()
    token = identity.admin_token()
    issuer = os.environ['BANK_ISSUER_BASE'] + '/realms/workforce'
    subjects = {}
    roles = {'reviewer': ['product_admin', 'case_reviewer', 'finance_adjuster', 'account_support', 'payment_inquiry', 'payment_operator'],
             'approver': ['product_approver', 'case_approver', 'finance_approver', 'security_approver', 'payment_approver'],
             'security': ['security_admin', 'operations_replay']}
    for username, grants in roles.items():
        subject = str(uuid.UUID(identity.call(token, 'workforce/users?exact=true&username=' + username)[0]['id']))
        subjects[username] = subject
        person = str(uuid.uuid5(uuid.NAMESPACE_URL, issuer + '/' + subject))
        sql(f"INSERT INTO controls.person_links VALUES('{issuer}','{subject}','{person}') ON CONFLICT DO NOTHING;")
        person = str(uuid.UUID(sql(f"SELECT person_id FROM controls.person_links WHERE issuer='{issuer}' AND subject='{subject}';")))
        for role in grants:
            grant = str(uuid.uuid5(uuid.NAMESPACE_URL, 'piggybank/synthetic-grant/' + person + '/' + role))
            sql(f"INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES('{grant}','{person}','{role}',now()+interval '365 days') ON CONFLICT(id) DO NOTHING;")
        if username in ['reviewer', 'approver']:
            capability = 'REVIEW' if username == 'reviewer' else 'APPROVE'
            for queue in ['controls-operations', 'support', 'payments-operations', 'finance-operations', 'data-operations']:
                sql(f"INSERT INTO controls.case_queue_members(queue,person_id,capability) VALUES('{queue}','{person}','{capability}') ON CONFLICT DO NOTHING;")
    sql("INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES('4f1a780b-3e31-48ef-a294-000000000003','SETTLEMENT','ACTIVE',false) ON CONFLICT DO NOTHING;")
    compose('run', '--rm', '-T', 'seed', data=json.dumps({'issuer': issuer, 'makerSubject': subjects['reviewer'], 'checkerSubject': subjects['approver']}))

if __name__ == '__main__':
    {'environment': create_environment, 'database': database, 'seed': seed}[sys.argv[1]]()
