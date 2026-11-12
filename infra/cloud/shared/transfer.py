#!/usr/bin/env python3
"""Transfer a release/backup using the VM identity. No credentials are printed."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import urllib.parse
import urllib.request

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('provider', choices=['gcp','aws'])
parser.add_argument('direction', choices=['upload','download'])
parser.add_argument('path', type=Path)
parser.add_argument('bucket')
parser.add_argument('object')
args=parser.parse_args()
if args.provider=='aws':
    remote=f's3://{args.bucket}/{args.object}'
    pair=[str(args.path),remote] if args.direction=='upload' else [remote,str(args.path)]
    subprocess.run(['aws','s3','cp',*pair,'--only-show-errors'],check=True)
else:
    request=urllib.request.Request('http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token',headers={'Metadata-Flavor':'Google'})
    with urllib.request.urlopen(request,timeout=10) as response:
        token=json.load(response)['access_token']
    headers={'Authorization':'Bearer '+token}
    bucket=urllib.parse.quote(args.bucket,safe='')
    key=urllib.parse.quote(args.object,safe='')
    if args.direction=='download':
        url=f'https://storage.googleapis.com/storage/v1/b/{bucket}/o/{key}?alt=media'
        with urllib.request.urlopen(urllib.request.Request(url,headers=headers),timeout=600) as response, args.path.open('wb') as target:
            shutil.copyfileobj(response,target)
    else:
        url=f'https://storage.googleapis.com/upload/storage/v1/b/{bucket}/o?uploadType=media&name={key}&ifGenerationMatch=0'
        def chunks():
            with args.path.open('rb') as stream:
                while data:=stream.read(1024*1024):
                    yield data
        headers.update({'Content-Type':'application/octet-stream','Content-Length':str(args.path.stat().st_size)})
        request=urllib.request.Request(url,data=chunks(),headers=headers,method='POST')
        with urllib.request.urlopen(request,timeout=600) as response:
            response.read()
print('Transfer completed.')
