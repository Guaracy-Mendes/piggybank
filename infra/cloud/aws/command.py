#!/usr/bin/env python3
"""Run a reviewable shell script over SSM and wait for its actual exit status."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('script', type=Path, help='Script containing no credentials; output is retained by SSM')
args=parser.parse_args()
region=os.environ['AWS_REGION']
instance=os.environ['AWS_INSTANCE_ID']
base=['aws','--region',region,'ssm']
result=json.loads(subprocess.check_output(base+['send-command','--instance-ids',instance,
    '--document-name','AWS-RunShellScript','--parameters',json.dumps({'commands':[args.script.read_text()], 'executionTimeout':['1800']}),
    '--output','json'],text=True))
command=result['Command']['CommandId']
print('SSM command: '+command,flush=True)
end=time.monotonic()+1900
while time.monotonic()<end:
    invocation=subprocess.run(base+['get-command-invocation','--command-id',command,'--instance-id',instance,'--output','json'],text=True,capture_output=True)
    if invocation.returncode:
        if 'InvocationDoesNotExist' not in invocation.stderr:
            raise SystemExit(invocation.stderr)
    else:
        state=json.loads(invocation.stdout)
        if state['Status'] not in ['Pending','InProgress','Delayed','Cancelling']:
            print(state.get('StandardOutputContent',''))
            print(state.get('StandardErrorContent',''))
            if state['Status']!='Success' or state.get('ResponseCode')!=0:
                raise SystemExit('SSM command failed: '+state['Status'])
            break
    time.sleep(5)
else:
    raise SystemExit('Local wait timed out; inspect the original SSM command before retrying')
