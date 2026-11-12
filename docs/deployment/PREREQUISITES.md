# Workstation prerequisites

Use an existing tool installation when it meets these versions: Terraform 1.10–1.x, Python 3.11+, Bash, Google Cloud CLI for GCP, AWS CLI v2 plus Session Manager plugin for AWS, and Docker Compose v2. Build containers as Linux/amd64; the cloud VMs are x86_64. The verified build uses the Rust/Node versions inside the delivered Dockerfile, so installing Rust/Node on the workstation is unnecessary for packaging.

The following commands are for an **Ubuntu 24.04 amd64 workstation** with sudo. For other systems use the linked official installation instructions and retain the same tool requirements. Do not run the installer blocks inside an already managed host without following that host's package-management policy.

## 1. Install basic tools

```sh
sudo apt-get update
sudo apt-get install -y ca-certificates curl gnupg unzip python3 make git openssh-client
python3 --version
uname -m
```

Expected: Python 3.11 or newer and `x86_64`. Obtain the complete Piggybank source tree in the intended workspace and run later commands from its root. The shared Dockerfile needs workspace manifests/locks, apps, crates, packages, contracts and migrations; the infrastructure folder alone cannot build application binaries.

## 2. Install Docker and Compose

```sh
sudo install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /tmp/piggybank-docker.asc
sudo install -m 0644 /tmp/piggybank-docker.asc /etc/apt/keyrings/docker.asc
printf '%s\n' \
  'deb [arch=amd64 signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu noble stable' \
  | sudo tee /etc/apt/sources.list.d/docker.list > /dev/null
sudo apt-get update
sudo apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
sudo systemctl enable --now docker
sudo usermod -aG docker "$(id -un)"
```

Sign out and sign back in before testing without sudo. Docker group members administer this host; use the trusted build/operator account. Then run:

```sh
docker version
docker compose version
docker run --rm --platform linux/amd64 hello-world
```

Expected: server accessible, Compose v2, and the test container exits successfully. This installs from Docker's signed apt repository. [Official Ubuntu installation](https://docs.docker.com/engine/install/ubuntu/).

The packager also supports `--runtime podman` on a configured Podman workstation. `make cloud-check` still needs a `docker compose` compatible CLI for configuration rendering, and the disposable acceptance fixture specifically uses Podman. Local Podman validation is optional for deploying an already verified archive.

## 3. Install Terraform

```sh
curl -fsSL https://apt.releases.hashicorp.com/gpg \
  | gpg --dearmor > /tmp/piggybank-hashicorp.gpg
sudo install -m 0644 /tmp/piggybank-hashicorp.gpg /usr/share/keyrings/hashicorp-archive-keyring.gpg
printf '%s\n' \
  'deb [arch=amd64 signed-by=/usr/share/keyrings/hashicorp-archive-keyring.gpg] https://apt.releases.hashicorp.com noble main' \
  | sudo tee /etc/apt/sources.list.d/hashicorp.list > /dev/null
sudo apt-get update
sudo apt-get install -y terraform
terraform version
```

Expected: Terraform at least 1.10 and below 2.0. The supplied mock tests and native S3 locking require this version range. The package was checked with 1.14.0; provider versions are sealed by the included lockfiles. [Official installation](https://developer.hashicorp.com/terraform/tutorials/aws-get-started/install-cli).

## 4. Install Google Cloud CLI for GCP

```sh
curl -fsSL https://packages.cloud.google.com/apt/doc/apt-key.gpg \
  | gpg --dearmor > /tmp/piggybank-google-cloud.gpg
sudo install -m 0644 /tmp/piggybank-google-cloud.gpg /usr/share/keyrings/cloud.google.gpg
printf '%s\n' \
  'deb [signed-by=/usr/share/keyrings/cloud.google.gpg] https://packages.cloud.google.com/apt cloud-sdk main' \
  | sudo tee /etc/apt/sources.list.d/google-cloud-sdk.list > /dev/null
sudo apt-get update
sudo apt-get install -y google-cloud-cli
gcloud version
```

Expected: `gcloud` and `gcloud storage` are available. Authenticate and choose the project in the [GCP runbook](gcp/RUNBOOK.md). [Official Google CLI installation](https://docs.cloud.google.com/sdk/docs/install-sdk).

## 5. Install AWS CLI and Session Manager plugin for AWS

```sh
mkdir -p .local/cloud/tools
curl -fLsS https://awscli.amazonaws.com/awscli-exe-linux-x86_64.zip \
  -o .local/cloud/tools/awscliv2.zip
unzip -q -o .local/cloud/tools/awscliv2.zip -d .local/cloud/tools
sudo .local/cloud/tools/aws/install --update
curl -fLsS \
  https://s3.amazonaws.com/session-manager-downloads/plugin/latest/ubuntu_64bit/session-manager-plugin.deb \
  -o .local/cloud/tools/session-manager-plugin.deb
sudo dpkg -i .local/cloud/tools/session-manager-plugin.deb
aws --version
session-manager-plugin --version
```

Expected: AWS CLI v2 and an installed Session Manager plugin. These URLs are the vendors' HTTPS installation endpoints. Apply vendor signature verification before installation where required by your workstation policy: [AWS CLI installation/signatures](https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html), [plugin installation](https://docs.aws.amazon.com/systems-manager/latest/userguide/install-plugin-debian-and-ubuntu.html), [plugin signature verification](https://docs.aws.amazon.com/systems-manager/latest/userguide/install-plugin-linux-verify-signature.html).

Authenticate through the [AWS runbook](aws/RUNBOOK.md). Cloud billing/project/account creation, organizational permissions and quota approvals remain prerequisites administered by the account owner; the deployment does not create or bypass them.

## 6. Check the delivered source package

```sh
make cloud-check
# Choose the cloud-specific runbook next; do not initialize a real backend until configured.
```

Expected: all deployment credential/configuration checks pass without a cloud account. Java/Graphviz are needed only to re-render all PRD/architecture diagrams, as described in the root README. The pre-rendered cloud SVG files require no diagram tooling.
