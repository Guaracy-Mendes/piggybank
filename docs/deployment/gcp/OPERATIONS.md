# GCP operations, recovery and removal

Run repository commands from its root. Retain the variables defined in the [deployment runbook](RUNBOOK.md). Host commands below run inside `gcloud compute ssh ... --tunnel-through-iap`; use `sudo -i` for maintenance. Set `BUCKET` and `ENVIRONMENT` inside the VM explicitly; workstation environment variables are not forwarded automatically.

## Inspect and restart

```sh
# Workstation: interactive IAP SSH session
gcloud compute ssh "$GCP_INSTANCE" --project "$GCP_PROJECT" --zone "$GCP_ZONE" --tunnel-through-iap
# VM:
sudo piggybank status
sudo piggybank logs worker
sudo journalctl -u piggybank-health.service -n 50 --no-pager
sudo df -h /srv/piggybank
sudo systemctl restart piggybank
sudo piggybank status
```

Expected: the existing data disk mounts and all health checks pass. Google startup scripts detect a previously accepted deployment and resume its recorded `current` release; they do not reinstall the original archive after an upgrade. The health timer records local smoke failures without an external alert destination. Arrange an operator notification/inspection process before unattended operation.

## Back up the complete stopped state

The VM identity can create objects only in its own `backups/<name>/` prefix and read releases/its own backup objects. It cannot delete backups. Use a timestamped object name; GCS uploads use `ifGenerationMatch=0` to prevent replacement.

```sh
# VM, as root:
BUCKET=YOUR_ARTIFACT_BUCKET
ENVIRONMENT=piggybank-eval
piggybank backup
BACKUP=$(find /srv/piggybank/backups -maxdepth 1 -name '*.tgz' -printf '%f\n' | sort | tail -n 1)
for suffix in '' .sha256 .manifest.json .release-path; do
  python3 /srv/piggybank/current/deployment/transfer.py gcp upload \
    "/srv/piggybank/backups/$BACKUP$suffix" "$BUCKET" "backups/$ENVIRONMENT/$BACKUP$suffix"
done
piggybank status
```

Expected: backup finishes after all containers stop, the trap restarts the stack, four objects exist and readiness returns. Backup contains passwords, HMAC keys, identities and immutable evidence; keep the bucket private. Assign retention explicitly and periodically inspect bucket generations. Preserve the matching original `releases/*.tgz` object and its independently recorded SHA256. Do not prune a release required by a retained backup.

Backups temporarily stop all services; schedule a maintenance window. This procedure does not define an automated backup frequency. A single data disk is a failure domain, so a local archive alone is insufficient. Copying live PG files or snapshotting only one subsystem does not reproduce this consistency guarantee.

## Upgrade application images and forward migrations

1. Run the normal source/application checks for the new change. Build a **new** release identifier with `package.py`, verify SHA256 and upload it with `gcloud storage cp --if-generation-match=0` as in the runbook.
2. Record the new release object and SHA256. Keep Terraform's initial bootstrap release inputs unchanged during routine application upgrades; those inputs describe provisioning. VM state records the accepted application release separately.
3. Announce the maintenance window. Make and upload a fresh stopped backup using the previous section; test its checksum.
4. Inside the VM run:

```sh
sudo bash /srv/piggybank/current/deployment/upgrade.sh gcp \
  YOUR_ARTIFACT_BUCKET releases/YOUR_NEW_RELEASE.tgz YOUR_NEW_64_CHARACTER_SHA256
sudo piggybank status
sudo cat /srv/piggybank/state/deployed-release-sha256
```

5. Expected: the installer verifies the archive and internal manifest, backs up and stops the previous stack, records its release path, loads the new images, applies embedded migrations and runs smoke checks. It skips role/identity fixture seeding after initialization and retains all existing keys, permissions and revocations. Verify both browser contexts again through IAP.
6. Upload the installer's pre-upgrade backup and retain deployment evidence. A failed migration/readiness check leaves an explicit failure for diagnosis; there is no automatic binary downgrade. Use the original command/operation identities for financial recovery.

Infrastructure changes still use `terraform plan`/`apply`; avoid replacing the VM or changing immutable bootstrap inputs as an application release mechanism. Image family resolution supplies Ubuntu 24.04 at provisioning; record the actual image/boot disk for qualification and apply OS updates through a separate maintenance window.

## Restore or roll back to a matched backup

This is a full evaluation environment rollback. It restores the simulator with the application, rolls back all activity after the backup cut, invalidates later sessions and requires a maintenance window. Preserve the failed state for investigation. Use an isolated recovery environment first when testing the procedure.

1. Select the four backup objects, exact original release object, backup SHA256 and original release SHA256 from the deployment record. Verify their origin outside the failed VM.
2. In a root VM session, download them to a root-only directory outside `state/`:

```sh
install -d -m 0700 /opt/piggybank-recovery
BUCKET=YOUR_ARTIFACT_BUCKET
ENVIRONMENT=piggybank-eval
BACKUP=YOUR_UTC_TIMESTAMP.tgz
RELEASE=YOUR_MATCHING_ORIGINAL_RELEASE
cd /opt/piggybank-recovery
for suffix in '' .sha256 .manifest.json .release-path; do
  python3 /srv/piggybank/current/deployment/transfer.py gcp download \
    "$BACKUP$suffix" "$BUCKET" "backups/$ENVIRONMENT/$BACKUP$suffix"
done
python3 /srv/piggybank/current/deployment/transfer.py gcp download \
  "$RELEASE.tgz" "$BUCKET" "releases/$RELEASE.tgz"
# Check against the independently recorded hashes, not just downloaded metadata.
```

3. Stop customer/operator changes and invoke the included restore helper with the two verified hashes:

```sh
bash /srv/piggybank/current/deployment/restore-state.sh \
  "/opt/piggybank-recovery/$BACKUP" VERIFIED_BACKUP_SHA256 \
  "/opt/piggybank-recovery/$RELEASE.tgz" VERIFIED_RELEASE_SHA256
piggybank status
```

4. Expected: exact backup/release manifests match before any stop; old state is renamed `state.before-restore-<timestamp>`, restored state retains original numeric owners/keys, the matching image is installed, and smoke passes. A mismatched release is rejected. Allow space for both the failed and restored state plus the archive.
5. Through tunneled browser sessions, verify retained cases, journal/report identities, evidence downloads, maker/checker relationships and original-identity payment/GL inquiry. Do not create replacement financial instructions for unknown outcomes. Record actual recovery elapsed time and backup age; no pre-qualified RTO/RPO is claimed.
6. After successful reconciliation, reopen the evaluation. Keep the failed state until the investigation/retention decision is complete.

If the VM boot disk is lost, preserve the Terraform-managed data disk. Provision a replacement VM in the same zone with that disk attached and the same artifact-read/backup-create identity. The bootstrap recognizes accepted state, reinstalls host service files from `current`, and resumes the mounted release. The old VM must be stopped/detached before a single-writer disk is attached to a replacement. Never format a restored disk or generate replacement keys against existing PG data. If the data disk is lost, create a blank disk and complete initial setup before the full stopped-state restore; its temporary state is preserved by the helper.

## Troubleshooting

| Symptom | Inspection | Expected correction |
| --- | --- | --- |
| IAP permission denied | User roles, OS Login and service-account usage; firewall source range | Correct designated operator grants; retain IAP-only ingress |
| Startup cannot download packages | Cloud NAT/router/subnet, DNS and approved outbound policy | Repair egress, inspect startup journal; retry the original bootstrap |
| Archive download denied or SHA mismatch | Bucket identity binding, immutable object and recorded digest | Correct object/IAM or use a new reviewed release; never bypass verification |
| Data device absent | Disk attachment and `/dev/disk/by-id/google-piggybank-data` | Attach the intended disk; do not initialize a substitute accidentally |
| Keycloak fails | `sudo piggybank logs keycloak`, persistent DB and original credential files | Restore consistent credentials/database; do not rotate by deleting `.env` |
| Readiness 503 | Dependency booleans, module logs, migration output/schema version | Repair the indicated dependency; run only the matching forward migration |
| Login loop or invalid redirect | Five tunnels, exact ports, independent browser profile | Restore `127.0.0.1` origins and clear that profile's stale sessions |
| Missing available cash | Finance evidence status and provider record | Supply valid synthetic evidence through its supported procedure; do not substitute ledger balance |
| Finance/payment unknown | Owned operations/cases and original provider inquiry | Follow domain recovery runbooks; preserve uncertain holds and exact command identities |
| Disk filling | `df`, Docker logs, releases, backup retention | Export verified backups; prune only archives/releases outside retention dependencies |

To retry a failed first bootstrap after fixing its dependency, inspect the original journal and run `sudo google_metadata_script_runner startup` in the VM. This reuses the verified input and persistent credentials. Do not reset initialized data or rerun synthetic authority seeding as a repair shortcut.

## Controlled teardown

1. Create, verify and export the final stopped backup and matching release. Record retained state/backup bucket names and ownership.
2. Set `protect_vm = false` and apply that reviewed Terraform change. Stop the application. VM termination protection and the data disk's Terraform `prevent_destroy` are independent.
3. For temporary suspension, stop the VM with `gcloud compute instances stop`; data disk, bucket versions and Cloud NAT remain billable.
4. For permanent removal, review the complete destroy plan. Full `terraform destroy` intentionally refuses while the data disk retains `prevent_destroy = true`. After the explicit retention decision, remove that lifecycle guard in a reviewed local change and create/review/apply a destroy plan. This deletes the data disk, so recovery then depends on the exported backup.
5. Remove no state/artifact bucket until all restore/retention obligations end. They were bootstrapped outside the stack. Remove project-level operator IAM grants only after confirming they are not shared with another environment. A dedicated disposable project can be retired through its own organization process.
6. Inspect billing/resources for retained bucket generations, disks/snapshots, NAT/router and abandoned addresses. Keep the final Terraform state and change evidence under retention.

## Independent clearing fixture publication

The default workstation tunnel does not include the provider. For trusted operator publication of outgoing or incoming/return synthetic clearing files, restart the tunnel bundle with `BANK_TUNNEL_PROVIDER=1` and supply the original installation Finance signing key through a private environment. Run the repository publisher on the workstation; its `--kind credits` mode selects the separate credit file store. Keep the optional tunnel disabled during normal portal use. Follow the [outgoing runbook](../../runbooks/finance-reconciliation.md) or [incoming/return runbook](../../runbooks/finance-credit-reconciliation.md); no public provider listener is created.

After restarting database/identity dependencies, verify a fresh customer and workforce login alongside API/worker health. Issuer discovery may be available while stale database connections are recovering. Preserve the original uncertain payment/GL references during recovery; a login failure does not determine their financial outcome.
