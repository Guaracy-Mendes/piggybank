#!/bin/bash
set -euo pipefail
cloud-init status --wait
systemctl status piggybank --no-pager
/usr/local/bin/piggybank status
