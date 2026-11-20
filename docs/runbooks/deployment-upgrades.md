# Deployment, upgrades and rollback

## Build and run

| Mode | How |
| --- | --- |
| Local development | [local development runbook](local-development.md) or `python3 scripts/local_cx.py prepare && python3 scripts/local_cx.py start` |
| Packaged | `infra/cloud/shared/package.py` builds the sealed release archive (images, Compose, bootstrap); `make cloud-check` verifies it. Images run as uid 10001 with read-only filesystems. See the cloud runbooks in `docs/deployment/`. |

## Rolling upgrade: expand, deploy, contract

1. **Check migrations.** Run `make migrations-check`. New migrations must be expand-only: new tables, columns with defaults, new functions or wrapper functions, widened constraints.
2. **Migrate first.** The `bank-migrate` job holds the only migration privilege. Running N-1 binaries keep working against the expanded schema.
3. **Deploy binaries.** Roll out the new API and worker. Readiness checks the expected schema, so a binary never serves against an older schema.
4. **Contract later.** Destructive cleanup goes in a later migration whose statements are marked `-- contract: <reason>`, applied after every N-1 binary is retired.
5. **Lock.** Run `make migrations-lock` after the migration reaches a shared environment.

## Rollback and forward recovery

- **Binary rollback.** Redeploy the previous image. Expand-only migrations keep the old binary compatible.
- **No schema rollback.** Never run a destructive "down" migration or a database reset. Fix forward with a new migration.
- **Financial safety.** Pause the rail (WP-21) to stop new submissions. Existing acknowledgements, returns and reconciliation keep processing. Unknown outcomes keep their holds and are recovered by inquiry (WP-23).

## Secrets interface

| Secret | Variable | Rules |
| --- | --- | --- |
| Session token key ring | `BANK_SESSION_KEYS_FILE` | `kid:base64(32 bytes)` per line; the first line seals and every line opens. Rotation needs no restart. |
| Issuer admin client secret | `BANK_IDENTITY_ADMIN_SECRET_FILE` | 16+ characters, mode 0600; re-read on every use |
| Metrics bearer token | `BANK_METRICS_TOKEN_FILE` | 32+ characters, mode 0600. Without it, `/metrics` is disabled. |

All three are validated at startup: unreadable, malformed or world-readable files stop the API. Production custody (KMS or HSM, workload identity) is WP-31.
