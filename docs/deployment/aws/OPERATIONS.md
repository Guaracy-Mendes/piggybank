# AWS operations, recovery and removal

Run workstation commands from the repository root with the region/instance variables from the [deployment runbook](RUNBOOK.md). Host snippets below run in an interactive Session Manager shell after `sudo -i`. Set VM-side values explicitly; workstation environment variables are not automatically forwarded.

## Inspect and restart

```sh
# Workstation:
aws ssm start-session --target "$AWS_INSTANCE_ID" --region "$AWS_REGION"
# VM:
sudo piggybank status
sudo piggybank logs worker
sudo journalctl -u piggybank-health.service -n 50 --no-pager
sudo df -h /srv/piggybank
sudo systemctl restart piggybank
sudo piggybank status
```

Expected: existing state and recorded `current` release resume without regenerating credentials or replaying authority seed. The health timer records smoke failures locally once a minute; it supplies no external paging destination. Docker logs rotate. Assign an operator inspection/notification process before unattended operation. Privileged commands can also run through [command.py](../../../infra/cloud/aws/command.py), which polls the original SSM command and reports its exit status. Keep secrets out of those retained command bodies and output.

## Create and export a complete stopped backup

The VM role can read release objects and read/write only its own `backups/<name>/` prefix. It has no backup deletion permission. S3 bucket versioning retains overwritten generations, but always use new timestamped backup names.

```sh
# VM as root:
export AWS_DEFAULT_REGION=us-east-1
BUCKET=YOUR_ARTIFACT_BUCKET
ENVIRONMENT=piggybank-eval
piggybank backup
BACKUP=$(find /srv/piggybank/backups -maxdepth 1 -name '*.tgz' -printf '%f\n' | sort | tail -n 1)
for suffix in '' .sha256 .manifest.json .release-path; do
  python3 /srv/piggybank/current/deployment/transfer.py aws upload \
    "/srv/piggybank/backups/$BACKUP$suffix" "$BUCKET" "backups/$ENVIRONMENT/$BACKUP$suffix"
done
piggybank status
```

Expected: all services stop before the archive cut; the exit trap starts them; readiness returns; four private encrypted/versioned S3 objects exist. The archive contains PostgreSQL/Keycloak, JetStream, evidence, original HMAC keys and passwords. Retain its checksum/manifest and the matching release artifact under restricted access. Define retention and backup frequency with the environment owner; neither is automated by this package. Stop-state backup creates a maintenance window.

A local archive on the same EBS volume is not disaster protection. EBS snapshots are a separate backup option; copying live subsystem files or taking an uncoordinated snapshot does not replace the documented stopped consistency cut. AWS makes the customer responsible for snapshot/backup scheduling. [Amazon EBS snapshot guidance](https://docs.aws.amazon.com/ebs/latest/userguide/ebs-snapshots.html).

## Upgrade images and apply forward migrations

1. Complete the application's change-specific validation. Build a new immutable release identifier, verify its SHA256 and upload it using step 3 of the deployment runbook. Preserve the old release and archive checksum.
2. Record the new S3 object/version and SHA256. Keep Terraform's initial bootstrap release inputs unchanged for routine upgrades; VM state records the accepted running release. Updating EC2 user data is not an application rollout mechanism.
3. Announce maintenance, create/export a stopped backup and verify its checksum. Use a root Session Manager shell for the installation:

```sh
export AWS_DEFAULT_REGION=us-east-1
bash /srv/piggybank/current/deployment/upgrade.sh aws \
  YOUR_ARTIFACT_BUCKET releases/YOUR_NEW_RELEASE.tgz YOUR_NEW_64_CHARACTER_SHA256
piggybank status
cat /srv/piggybank/state/deployed-release-sha256
```

4. Expected: SHA256 and internal manifest verified, prior state backed up/stopped, previous release recorded, images loaded, embedded forward migrations applied and smoke passed. Existing credentials, HMAC archives and current authority/revocation records persist. Initialization seeding does not run again.
5. Upload the installer's pre-upgrade backup, verify both browser contexts through the five tunnels and record the result. There is no automatic downgrade after a migration failure. Inspect the original error/operation and use the matched restore procedure if rollback is required.

For OS/infrastructure changes use reviewed Terraform plans and a separate maintenance window. Pin the accepted AMI in `ami_id` before reproducible VM replacement. Confirm the same-AZ, single-writer volume attachment and SSM instance-scoped operator policy after replacing an instance.

## Restore and application rollback

This is a complete synthetic-environment rollback to one stopped cut. It discards later activity, restores the simulator with Core and invalidates sessions created later. First rehearse in a separate recovery environment. Preserve failed state for diagnosis and retain independently recorded checksums.

1. Choose the backup timestamp, its four sidecar objects, matching original release object, backup SHA256 and original release SHA256. Determine object versions if a name was ever reused. For such cases download the exact recorded versions with `aws s3api get-object --version-id ...` rather than the helper's latest-version download.
2. In a root Session Manager shell, download to a root-only recovery directory outside `state/`:

```sh
export AWS_DEFAULT_REGION=us-east-1
install -d -m 0700 /opt/piggybank-recovery
BUCKET=YOUR_ARTIFACT_BUCKET
ENVIRONMENT=piggybank-eval
BACKUP=YOUR_UTC_TIMESTAMP.tgz
RELEASE=YOUR_MATCHING_ORIGINAL_RELEASE
cd /opt/piggybank-recovery
for suffix in '' .sha256 .manifest.json .release-path; do
  python3 /srv/piggybank/current/deployment/transfer.py aws download \
    "$BACKUP$suffix" "$BUCKET" "backups/$ENVIRONMENT/$BACKUP$suffix"
done
python3 /srv/piggybank/current/deployment/transfer.py aws download \
  "$RELEASE.tgz" "$BUCKET" "releases/$RELEASE.tgz"
```

3. Stop customer/operator activity. Verify origin and independent hashes, then run:

```sh
bash /srv/piggybank/current/deployment/restore-state.sh \
  "/opt/piggybank-recovery/$BACKUP" VERIFIED_BACKUP_SHA256 \
  "/opt/piggybank-recovery/$RELEASE.tgz" VERIFIED_RELEASE_SHA256
piggybank status
```

4. Expected: full manifest equality is checked before stopping anything; a mismatched release is rejected. Existing state is renamed `state.before-restore-<timestamp>`, the original state/credentials/owners are restored together, and matching images/migrations/startup pass smoke. Reserve enough disk for both states and the archive.
5. Reconnect tunnels and verify cases/evidence, retained journal/report identities, independent approvals and original-identity inquiry. Use [payment recovery](../../runbooks/payment-recovery.md) and [Finance recovery](../../runbooks/finance-recovery.md). Do not reissue financial instructions to compensate for an unknown outcome. Record observed recovery elapsed time and actual backup age; no qualified RTO/RPO is claimed.
6. Reopen the evaluation after reconciliation. Retain failed state and export recovery evidence according to the investigation/retention decision.

If the root disk is lost, preserve the Terraform-managed EBS data volume. Stop/detach the old instance and attach that single-writer volume to a replacement in the same AZ with the same artifact/backup role. The bootstrap restores systemd host files from `current` and resumes recorded state. Regenerate/re-attach the instance-scoped operator policy for the replacement. Never format restored storage or generate new HMAC keys against old PG data. For a lost data volume, provision a fresh environment and perform the full matched state restore after its temporary initialization.

## Troubleshooting

| Symptom | Inspect | Correction |
| --- | --- | --- |
| SSM offline | Instance profile, agent, DNS, subnet route/IGW and outbound HTTPS | Repair managed-node connectivity; do not open SSH/public application ingress |
| SSM access denied | Intended account/profile, generated operator policy and document/instance ARN | Attach the reviewed policy to the designated operator; regenerate after replacement |
| Cloud-init failed before service creation | `aws ec2 get-console-output` or interactive `sudo tail /var/log/cloud-init-output.log` | Fix the actual download/disk/credential dependency before retrying |
| Data device absent | `ls -l /dev/disk/by-id/`, EBS attachment status and AZ | Attach the exact volume; retain its filesystem and contents |
| Release GetObject denied | VM role/prefix, object and bucket TLS policy | Correct IAM or immutable key; never disable SHA256 verification |
| AMI lookup/instance capacity fails | Region/AZ, Canonical parameter and architecture, approved quota | Select an approved available x86_64 shape/AMI and review the new plan |
| Readiness 503 | API/worker booleans, Docker logs and migration schema version | Repair indicated dependency with matching forward migrations |
| Login redirect/session failure | Five tunnels, unchanged ports and independent browser profile | Restore loopback origin/issuer consistency; clear stale profile sessions |
| Unavailable cash or uncertain payment | Provider evidence/cases and original-identity inquiry | Follow financial recovery procedures; retain holds and explicit missing status |
| Disk filling | Backups, retained releases, container log rotation and failed states | Export and verify first; prune only data outside agreed retention dependencies |

To retry first boot after its underlying problem is fixed, inspect cloud-init and the mounted disk. The original script is retained under `/var/lib/cloud/instance/scripts/part-001`; execute it with `sudo bash` only after reviewing that original file. The installer reuses persistent credentials and never needs `cloud-init clean` or volume deletion. A previously accepted state resumes its recorded release instead of reinstalling the initial artifact.

## Controlled suspension and removal

1. Create/export/verify the final stopped backup and matching release. Record the artifact/state buckets and retained object versions.
2. Temporary suspension: `aws ec2 stop-instances --instance-ids "$AWS_INSTANCE_ID" --region "$AWS_REGION"`. This retains EBS/storage costs. Restart through the provisioning/operations process, wait for SSM, then smoke before reopening.
3. For permanent teardown set `protect_vm = false` and apply that reviewed change, then stop the application. EC2 termination protection and Terraform data-volume `prevent_destroy` are separate safeguards.
4. Review the complete destroy plan. Full destroy intentionally refuses while the EBS data volume retains `prevent_destroy = true`. After the explicit retention decision, remove that guard in a reviewed local change, plan destroy and apply the exact plan. Removing the data volume makes the exported backup your recovery source.
5. Keep backend/artifact buckets until all retention/restore obligations end; they are outside this stack and are not removed by destroy. Remove the dedicated inline operator policy only after the instance is retired and the policy is not reused. Retain final state/change evidence.
6. Inspect the account for remaining EBS snapshots/volumes, S3 versions, IPv4 addresses and other stack resources. A stopped or terminated EC2 instance is not proof that every billable artifact is gone.

## Independent clearing fixture publication

The default workstation tunnel does not include the provider. For trusted operator publication of outgoing or incoming/return synthetic clearing files, restart the tunnel bundle with `BANK_TUNNEL_PROVIDER=1` and supply the original installation Finance signing key through a private environment. Run the repository publisher on the workstation; its `--kind credits` mode selects the separate credit file store. Keep the optional tunnel disabled during normal portal use. Follow the [outgoing runbook](../../runbooks/finance-reconciliation.md) or [incoming/return runbook](../../runbooks/finance-credit-reconciliation.md); no public provider listener is created.

After restarting database/identity dependencies, verify a fresh customer and workforce login alongside API/worker health. Issuer discovery may be available while stale database connections are recovering. Preserve the original uncertain payment/GL references during recovery; a login failure does not determine their financial outcome.
