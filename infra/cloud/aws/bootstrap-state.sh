#!/bin/bash
set -euo pipefail
: "${AWS_REGION:?set AWS_REGION}"
: "${STATE_BUCKET:?set globally unique STATE_BUCKET}"
: "${ARTIFACT_BUCKET:?set globally unique ARTIFACT_BUCKET}"
account=$(aws sts get-caller-identity --query Account --output text)
for bucket in "$STATE_BUCKET" "$ARTIFACT_BUCKET"; do
  if ! aws s3api head-bucket --bucket "$bucket" --expected-bucket-owner "$account" >/dev/null 2>&1; then
    if [[ "$AWS_REGION" == us-east-1 ]]; then
      aws s3api create-bucket --bucket "$bucket" --region "$AWS_REGION" > /dev/null
    else
      aws s3api create-bucket --bucket "$bucket" --region "$AWS_REGION" --create-bucket-configuration "LocationConstraint=$AWS_REGION" > /dev/null
    fi
  fi
  aws s3api head-bucket --bucket "$bucket" --expected-bucket-owner "$account"
  location=$(aws s3api get-bucket-location --bucket "$bucket" --query LocationConstraint --output text)
  [[ "$location" == None ]] && location=us-east-1
  [[ "$location" == "$AWS_REGION" ]] || { echo 'Bucket is outside intended region'; exit 1; }
  aws s3api put-public-access-block --bucket "$bucket" --public-access-block-configuration BlockPublicAcls=true,IgnorePublicAcls=true,BlockPublicPolicy=true,RestrictPublicBuckets=true
  aws s3api put-bucket-versioning --bucket "$bucket" --versioning-configuration Status=Enabled
  aws s3api put-bucket-encryption --bucket "$bucket" --server-side-encryption-configuration '{"Rules":[{"ApplyServerSideEncryptionByDefault":{"SSEAlgorithm":"AES256"},"BucketKeyEnabled":false}]}'
  aws s3api put-bucket-policy --bucket "$bucket" --policy "{\"Version\":\"2012-10-17\",\"Statement\":[{\"Sid\":\"RequireTLS\",\"Effect\":\"Deny\",\"Principal\":\"*\",\"Action\":\"s3:*\",\"Resource\":[\"arn:aws:s3:::$bucket\",\"arn:aws:s3:::$bucket/*\"],\"Condition\":{\"Bool\":{\"aws:SecureTransport\":\"false\"}}}]}"
done
printf 'bucket = "%s"\nkey = "piggybank/evaluation.tfstate"\nregion = "%s"\nencrypt = true\nuse_lockfile = true\n' "$STATE_BUCKET" "$AWS_REGION" > infra/cloud/aws/backend.hcl
