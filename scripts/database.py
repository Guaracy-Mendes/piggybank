#!/usr/bin/env python3
"""Provision synthetic local runtime roles; no real provider credentials are involved."""
import os
import subprocess

modules = ["platform", "identity", "core", "controls", "journeys", "payments", "finance", "documents", "data", "simulator"]
statements=[]
for module in modules:
    statements.append(f"""DO $$ BEGIN
      IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='bank_{module}') THEN
        CREATE ROLE bank_{module} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT PASSWORD 'synthetic-{module}-only';
      END IF;
    END $$;""")
subprocess.run([os.environ.get("CONTAINER_RUNTIME","podman"),"exec","-i","piggybank-postgres-1","psql","-U","dev_admin","-d","piggybank","-v","ON_ERROR_STOP=1"],input="\n".join(statements),text=True,check=True,stdout=subprocess.DEVNULL)
print("Synthetic module runtime roles provisioned; no schema ownership granted.")
