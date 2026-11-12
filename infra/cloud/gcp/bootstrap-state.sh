#!/bin/bash
set -euo pipefail
: "${GCP_PROJECT:?set GCP_PROJECT}"
: "${GCP_REGION:?set GCP_REGION}"
: "${STATE_BUCKET:?set globally unique STATE_BUCKET}"
: "${ARTIFACT_BUCKET:?set globally unique ARTIFACT_BUCKET}"
gcloud services enable storage.googleapis.com serviceusage.googleapis.com --project "$GCP_PROJECT"
for bucket in "$STATE_BUCKET" "$ARTIFACT_BUCKET"; do
  if ! gcloud storage buckets describe "gs://$bucket" --project "$GCP_PROJECT" > /dev/null 2>&1; then
    gcloud storage buckets create "gs://$bucket" --project "$GCP_PROJECT" --location "$GCP_REGION" --uniform-bucket-level-access --public-access-prevention
  fi
  # Stop if a pre-existing global name belongs to another project.
  actual=$(gcloud storage buckets describe "gs://$bucket" --format='value(projectNumber)')
  expected=$(gcloud projects describe "$GCP_PROJECT" --format='value(projectNumber)')
  [[ "$actual" == "$expected" ]] || { echo 'Bucket is outside intended project'; exit 1; }
  gcloud storage buckets update "gs://$bucket" --versioning --uniform-bucket-level-access --public-access-prevention
 done
printf 'bucket = "%s"\nprefix = "piggybank/evaluation"\n' "$STATE_BUCKET" > infra/cloud/gcp/backend.hcl
