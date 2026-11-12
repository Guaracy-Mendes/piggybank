#!/bin/bash
set -euo pipefail
cd /srv/piggybank/current/deployment
install -m 0750 ops.sh /usr/local/bin/piggybank
cat > /etc/systemd/system/piggybank.service <<'UNIT'
[Unit]
Description=Piggybank synthetic cloud evaluation
Requires=docker.service
After=docker.service network-online.target
Wants=network-online.target
RequiresMountsFor=/srv/piggybank
[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/usr/local/bin/piggybank start
ExecStop=/usr/local/bin/piggybank stop
TimeoutStartSec=600
TimeoutStopSec=180
[Install]
WantedBy=multi-user.target
UNIT
install -m 0644 piggybank-health.service piggybank-health.timer /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now piggybank-health.timer
systemctl enable piggybank
