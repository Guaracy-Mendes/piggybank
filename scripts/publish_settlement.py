#!/usr/bin/env python3
"""Publish an explicit bounded synthetic clearing fixture; never prints provider keys."""
import argparse
import json
import os
from pathlib import Path
import urllib.error
import urllib.parse
import urllib.request

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('file',type=Path)
parser.add_argument("--kind", choices=("outgoing", "credits", "set"), default="outgoing")
args=parser.parse_args()
if os.environ.get('BANK_PROFILE','local')!='local':
    raise SystemExit('Only the synthetic local profile is implemented')
base=os.environ.get('BANK_FINANCE_PROVIDER_BASE','http://127.0.0.1:4182').rstrip('/')
url=urllib.parse.urlsplit(base)
if url.scheme!='http' or url.hostname not in ['127.0.0.1','localhost','::1'] or url.username or url.password or url.query or url.fragment or url.path:
    raise SystemExit('Synthetic clearing publisher requires a loopback provider base')
with args.file.open('rb') as stream:
    body=stream.read(60001)
if len(body)>60000:
    raise SystemExit('Clearing file exceeds its 60,000 byte bound')
json.loads(body)
key=os.environ.get('BANK_FINANCE_SIGNING_KEY','synthetic-finance-authentication-key-only-2026')
endpoint={'outgoing':'/admin/settlement-files','credits':'/admin/credit-settlement-files','set':'/admin/settlement-sets'}[args.kind]
request=urllib.request.Request(base+endpoint,data=body,headers={'Content-Type':'application/json','X-Finance-Key':key},method='POST')
class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        return None

try:
    with urllib.request.build_opener(NoRedirect()).open(request,timeout=5) as response:
        print('Synthetic clearing publication HTTP '+str(response.status))
except urllib.error.HTTPError as error:
    raise SystemExit('Synthetic clearing publication rejected: HTTP '+str(error.code)) from None
