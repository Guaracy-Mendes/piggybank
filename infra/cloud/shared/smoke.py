#!/usr/bin/env python3
"""Read-only live smoke; no fabricated financial/provider success claims."""
import json
import os
import urllib.error
import urllib.request

def read(url):
    offset = int(os.environ.get('BANK_CLOUD_SMOKE_PORT_OFFSET', '0'))
    if offset:
        from urllib.parse import urlsplit
        parsed = urlsplit(url)
        url = parsed._replace(netloc=f'127.0.0.1:{parsed.port + offset}').geturl()
    with urllib.request.urlopen(url, timeout=5) as response:
        return response.read()

for port in [4180, 4181]:
    for probe, expected in [('live', 'alive'), ('ready', 'ready')]:
        url = f'http://127.0.0.1:{port}/health/{probe}'
        result = json.loads(read(url))
        assert result['status'] == expected, (url, result)
        if probe == 'ready':
            assert result['dependencies'] and all(result['dependencies'].values())
for port in [4300, 4301]:
    assert b'<html' in read(f'http://127.0.0.1:{port}/').lower()
for port, path in [(4300, '/api/v1/staff/access'), (4301, '/api/v1/customer/accounts'),
                   (4300, '/auth/workforce/login'), (4301, '/auth/customer/login'),
                   (4300, '/api/v1/%73taff/access'), (4301, '/api/v1/staff/../customer/accounts')]:
    try:
        read(f'http://127.0.0.1:{port}{path}')
        raise AssertionError(f'Channel boundary bypass at {port}{path}')
    except urllib.error.HTTPError as error:
        assert error.code == 403, error.code
assert json.loads(read('http://127.0.0.1:4182/health/live'))['status'] == 'alive'
print('API/worker dependencies, two static portals, provider and six channel-denial checks passed.')
