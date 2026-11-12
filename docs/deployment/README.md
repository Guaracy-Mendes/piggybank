# Cloud deployment packages

These two packages deploy the **implemented synthetic evaluation system**. They preserve the current local-only application profile, loopback OIDC issuer and provider endpoints, separate PostgreSQL module authorities and immutable financial records. A private operator tunnel makes the VM's loopback services available on the same workstation ports.

These packages include the grouped-clearing workbench the independently approved periodic preview-policy workspace, funding-only corrections and native ledger basis observations. The current release examples target schema 51; preview policies do not create customer charges or interest journals.

| Package | Step-by-step deployment | Day-two operations | Infrastructure |
| --- | --- | --- | --- |
| Google Cloud | [GCP runbook](gcp/RUNBOOK.md) | [GCP operations](gcp/OPERATIONS.md) | [Terraform and scripts](../../infra/cloud/gcp/) |
| Amazon Web Services | [AWS runbook](aws/RUNBOOK.md) | [AWS operations](aws/OPERATIONS.md) | [Terraform and scripts](../../infra/cloud/aws/) |

Start with the [workstation prerequisites](PREREQUISITES.md) when installing the tools. Both sets use the [shared container, lifecycle and backup files](../../infra/cloud/shared/). The [validation record](VALIDATION.md) distinguishes executable local checks from live cloud qualification. No cloud resources are created by documentation validation.

## Supported deployment and limits

One Ubuntu 24.04 amd64 VM hosts API, worker, synthetic provider, two compiled React portals, PostgreSQL 18.4, Keycloak 26.5 and NATS JetStream 2.12. Linux host networking deliberately preserves the application's loopback assumptions. Each dependency and application listener stays on `127.0.0.1`. The VM's host namespace, root operators and Docker administrators are trusted. Neither topology provides host-level tenant isolation or high availability.

The application explicitly rejects `BANK_PROFILE` values other than `local`. Cookies and issuer setup are designed for loopback HTTP. These files therefore provide cloud-hosted evaluation rather than a public banking launch. Keep customer data synthetic and retain the `SYNTHETIC`/TST labels. Public HTTPS domains, Secure cookies, qualified real providers, managed-database adapters, multi-node storage/broker operation, capacity, compliance, alert delivery and disaster-recovery objectives require subsequent implementation and qualification. The [execution board](../implementation/WORK_PACKAGES.md) remains authoritative for product completion; this deployment package does not complete those packages.

GCP uses a VM without an external IP, Cloud NAT for outbound installation traffic, OS Login and an IAP-only SSH firewall. AWS uses an outbound-capable public subnet/address with an empty ingress security group, IMDSv2 and Session Manager. These are different network designs; the AWS address does not expose application listeners.

## Release and persistence contract

- Build a secret-free `.tgz` with [package.py](../../infra/cloud/shared/package.py). It includes two Docker image archives, deployment code, identity realm templates, a file manifest and SHA256 checks. Reuse one immutable archive in either cloud. Source and dependency lockfiles are build inputs; Rust/Node builder tags follow the repository's pinned versions. Runtime dependency images retain their existing digests. The produced image/archive checksums seal each actual build; base-image tags alone are not a reproducible supply-chain attestation.
- Terraform manages networking, VM identity, boot/data disks and artifact access. Its backend uses a separately created private, versioned bucket. No generated application passwords are written to Terraform inputs or state. Plans and state remain sensitive infrastructure records.
- `/srv/piggybank` is a separate encrypted, protected data disk. `state/` contains PostgreSQL (including Keycloak), JetStream, evidence files, original HMAC keys, DB/client credentials and the initialization marker. `releases/` contains installed image archives and manifests; `current` identifies the running release. Root-only `.env` contains migration/admin credentials; `runtime.env` contains module/client/provider credentials. Only runtime credentials reach application containers.
- First initialization provisions roles, applies all embedded forward migrations, configures identity, links distinct reviewer/approver people, then uses the existing independent publication commands to seed synthetic product, chart, terms and rail policy. Runtime modules do not receive migration credentials. Later upgrades skip identity/authority seeding and preserve revocations.
- Database migrations are forward-only and checksum protected. Schema/version readiness rejects an incompatible binary. Application rollback after migration requires restoring the matching complete backup, never editing applied migrations or replaying payment commands.
- An upgrade stops the old stack and records a complete stopped backup before migrating. It retains previous release information. There is an evaluation maintenance window; no zero-downtime claim is made.
- A stopped backup captures PG, broker, evidence and credentials together. Retain its manifest, SHA256 and original release artifact off-host. Disk snapshots alone are not the documented application-consistent recovery procedure. No automated backup schedule or RPO/RTO is asserted.

## Ports and access

| Port | Purpose | Forward to workstation? |
| --- | --- | --- |
| 4180 | API and both BFF callback paths | Yes |
| 4181 | Worker health | Yes |
| 4300 | Customer static portal and scoped proxy | Yes |
| 4301 | Staff static portal and scoped proxy | Yes |
| 18280 | Keycloak issuer and login | Yes |
| 4182 | Synthetic controls, rails, GL, liquidity and clearing files | Optional trusted-operator fixture tunnel; disabled by default |
| 15432 | PostgreSQL | No |
| 14222 / 18222 | JetStream / broker monitoring | No |
| 9000 | Keycloak management | No |

Use `127.0.0.1`, retain these exact browser ports and keep all five application/issuer tunnels running. Set `BANK_TUNNEL_PROVIDER=1` when invoking either tunnel script to add the provider loopback port for authenticated operator fixture publication. It retains loopback binding and the existing host/IAP/SSM access boundary; no public ingress is added. Leave this optional tunnel disabled for normal portal use. Stop a local Piggybank runtime before tunneling; `make dev-down` retains its data. A cloud deployment does not operate as a different browser origin, so use a separate browser profile when switching between environments. Test one cloud at a time on a workstation.

## Common validation and operations

From the repository root:

```sh
make cloud-check                         # Python, shell and Compose checks; no account needed
terraform -chdir=infra/cloud/gcp init -backend=false
terraform -chdir=infra/cloud/gcp validate
terraform -chdir=infra/cloud/gcp test      # Mock provider; creates no resources
terraform -chdir=infra/cloud/aws init -backend=false
terraform -chdir=infra/cloud/aws validate
terraform -chdir=infra/cloud/aws test
# Full disposable image/dependency acceptance, after packaging; requires Podman:
python3 tests/deployment/runtime.py cloud-20261111-01
```

`terraform init -backend=false` is for an unused validation checkout. Use the cloud runbook's backend initialization for real deployments. Keep the generated provider lockfiles. Image acceptance verifies the selected `.tgz` checksum, complete manifest and member hashes, rejects unsafe archive entries, loads its exact image archive and runs its packaged deployment code. It does not substitute files from the current checkout. The build uses `npm ci --ignore-scripts`, locked Cargo inputs, compiled static portals and a non-root application runtime. `make cloud-check` requires Python 3.11+, Docker Compose v2 and Bash. Terraform 1.10+ is required for native S3 lockfiles. [HashiCorp documents GCS locking](https://developer.hashicorp.com/terraform/language/backend/gcs) and [S3 `use_lockfile`](https://developer.hashicorp.com/terraform/language/backend/s3).

On either VM, use `sudo piggybank status`, `sudo piggybank logs worker`, `sudo piggybank backup`, `sudo systemctl stop piggybank` and `sudo systemctl start piggybank`. Health runs once per minute through `piggybank-health.timer`; inspect `journalctl -u piggybank-health.service`. It records local failures and supplies no external paging. Container logs rotate at five 10 MB files per service. Keep health checks distinct from settlement confirmation, ledger reconciliation and product readiness.

Preserve uncertain payment/GL identities during outages. Resume original-identity inquiry through the [payment recovery](../runbooks/payment-recovery.md), [Finance recovery](../runbooks/finance-recovery.md), [reporting](../runbooks/finance-reporting.md) and [report revision](../runbooks/finance-revisions.md) procedures. A backup restore rolls the simulator back with Core; this is safe only for the included synthetic providers. Real external provider outcomes would require independent post-restore reconciliation.

The archive recorded in the 2026-11-03 validation is a historical migration-44 checkpoint. Current grouped clearing reconciliation requires migration 47 (single-file outgoing and incoming/return matching require 45/46); the runbook examples now use a fresh release identifier and build current source. Do not reuse the historical image archive to deploy the newer implementation.
