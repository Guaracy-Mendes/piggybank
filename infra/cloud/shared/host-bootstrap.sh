# Included verbatim in provider startup templates. Ubuntu 24.04, x86_64 only.
set -euo pipefail
umask 077
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y ca-certificates curl gnupg python3 unzip util-linux
install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc
cat > /etc/apt/sources.list.d/docker.sources <<'APT'
Types: deb
URIs: https://download.docker.com/linux/ubuntu
Suites: noble
Components: stable
Architectures: amd64
Signed-By: /etc/apt/keyrings/docker.asc
APT
apt-get update
apt-get install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin
systemctl enable --now docker
for attempt in $(seq 1 120); do
  [[ -b "$BANK_DATA_DEVICE" ]] && break
  sleep 2
done
[[ -b "$BANK_DATA_DEVICE" ]] || { echo 'Data disk absent'; exit 1; }
# Only the designated whole, unformatted data disk can be initialized.
root_source=$(findmnt -n -o SOURCE /)
[[ "$(readlink -f "$BANK_DATA_DEVICE")" != "$(readlink -f "$root_source")" ]] || exit 1
fs_type=$(blkid -s TYPE -o value "$BANK_DATA_DEVICE" || true)
if [[ -z "$fs_type" ]]; then
  [[ -z "$(wipefs --no-act --noheadings --output TYPE "$BANK_DATA_DEVICE")" ]] || { echo 'Unexpected disk signature'; exit 1; }
  mkfs.ext4 -L piggybank-data "$BANK_DATA_DEVICE"
elif [[ "$fs_type" != ext4 ]]; then
  echo 'Expected ext4 data disk; refusing to format an existing filesystem'; exit 1
fi
mkdir -p /srv/piggybank
volume_uuid=$(blkid -s UUID -o value "$BANK_DATA_DEVICE")
if ! findmnt --fstab --mountpoint /srv/piggybank > /dev/null; then
  printf 'UUID=%s /srv/piggybank ext4 defaults 0 2\n' "$volume_uuid" >> /etc/fstab
fi
mountpoint -q /srv/piggybank || mount /srv/piggybank
[[ "$(findmnt -n -o UUID --mountpoint /srv/piggybank)" == "$volume_uuid" ]] || { echo 'Wrong data filesystem mounted'; exit 1; }
chmod 0700 /srv/piggybank
mkdir -p /opt/piggybank-bootstrap
