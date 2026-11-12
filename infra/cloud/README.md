# Cloud deployment files

Start with the [deployment overview](../../docs/deployment/README.md).

| Environment | Infrastructure | Step-by-step runbook | Operations |
| --- | --- | --- | --- |
| GCP | [gcp/](gcp/) | [Deploy](../../docs/deployment/gcp/RUNBOOK.md) | [Operate and restore](../../docs/deployment/gcp/OPERATIONS.md) |
| AWS | [aws/](aws/) | [Deploy](../../docs/deployment/aws/RUNBOOK.md) | [Operate and restore](../../docs/deployment/aws/OPERATIONS.md) |

[shared/](shared/) contains the multi-stage Dockerfile, exact Compose topology, static portal proxy, package builder, credential/database/identity bootstrap, verified installer, lifecycle commands, health timer, backup transfer and matched restore scripts. Both providers consume the same image release. Provider directories contain locked Terraform, mock-plan checks, backend/environment examples, state bootstrap and operator tunnel files. AWS also supplies an instance-scoped operator policy and a synchronous Run Command helper.

Run `make cloud-check` before configuring live backend files. The current application is synthetic and loopback-only; use the IAM-protected operator tunnels. Do not apply these as a public production deployment.
