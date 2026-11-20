# Backup and restore runbook

This runbook is for the local synthetic profile (WP-29). Production backup policy, retention and geography are DEC-06, DEC-07 and WP-30/31 inputs.

## Back up

```sh
podman exec piggybank-postgres-1 pg_dump -U dev_admin -Fc -f /tmp/piggybank.dump piggybank
podman cp piggybank-postgres-1:/tmp/piggybank.dump .local/backups/piggybank-$(date -u +%Y%m%dT%H%M%SZ).dump
```

The dump contains synthetic data only. It does not contain issuer (Keycloak) data or secret files.

## Restore into a new database (never over the original)

```sh
podman exec piggybank-postgres-1 psql -U dev_admin -d postgres -c "CREATE DATABASE piggybank_restore OWNER bank_migrator"
podman exec piggybank-postgres-1 pg_restore -U dev_admin --exit-on-error -d piggybank_restore /tmp/piggybank.dump
```

## Verify before any financial write resumes

Run these against the restored database:

1. **Migrations.** `_sqlx_migrations` count and checksums equal the source and `contracts/migrations.lock.json`.
2. **Balanced journals.** `SELECT count(*) FROM (SELECT batch_id FROM core.journal_entries GROUP BY batch_id HAVING sum(delta)<>0) u` returns 0.
3. **Booked balances.** Every `core.ledger_accounts.booked` equals the sum of its journal entries.
4. **Held amounts.** Every `held` equals the sum of its ACTIVE holds.
5. **Records.** Payment intents and states, audit event count and audit chain heads equal the source at backup time.
6. **Read-only start.** Start the API against the restored database, check readiness, and compare a customer balance with the expected value. Keep the rail paused and the worker stopped until reconciliation explains every uncertain operation (WP-23 recovery and reconciliation).

`tests/integration/deployment/operations.py` performs this drill automatically and records dump and restore durations.

## Switch over

Point services at the restored database only after verification. Resume the worker, then lift the rail pause through the independent resume control. Never run two writers against different copies.
