#!/usr/bin/env python3
"""AT-27 branches: limits, contract/status behavior and shared instance quota."""
import json
import hashlib
import hmac
import base64
from datetime import datetime, timezone, timedelta
import os
import re
from pathlib import Path
import subprocess
import time
import urllib.error
import urllib.request
import uuid

ROOT=Path(__file__).resolve().parents[2]

def call(port,path,method="GET",body=None,headers=None):
    request=urllib.request.Request(f"http://127.0.0.1:{port}{path}",data=body,method=method,headers=headers or {})
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self,*args,**kwargs):return None
    try:response=urllib.request.build_opener(NoRedirect()).open(request,timeout=10)
    except urllib.error.HTTPError as error:response=error
    with response:return response.status,response.read(),response.headers

def start(port,limit=500,scope=None):
    env=dict(os.environ,BANK_BIND=f"127.0.0.1:{port}",BANK_RATE_LIMIT=str(limit),BANK_RATE_SCOPE=scope or str(uuid.uuid4()))
    process=subprocess.Popen([str(ROOT/"target/debug/bank-api")],cwd=ROOT,env=env,stdout=subprocess.DEVNULL)
    for _ in range(40):
        try:
            if call(port,"/health/ready")[0]==200:return process
        except OSError:pass
        time.sleep(.25)
    process.terminate();process.wait(timeout=10);raise AssertionError("API readiness timeout")

def main():
    processes=[]
    try:
        processes.append(start(4190))
        spec=json.loads((ROOT/"contracts/openapi/implemented.json").read_text())
        for path,methods in spec["paths"].items():
            for method in methods:
                sample=re.sub(r'\{(?:id|challengeId|policyId|offerId)\}',str(uuid.uuid4()),path.replace("{trust}","customer"))
                status,body,headers=call(4190,sample,method.upper())
                assert str(status) in methods[method]["responses"],(path,method,status)
                assert headers["Cache-Control"]=="no-store"
                uuid.UUID(headers["x-request-id"])
        for path in ["/api","/api/","/api/v9/unknown"]:
            status,body,headers=call(4190,path,headers={"X-Request-Id":"invalid-request-id"})
            assert status==404 and json.loads(body)["error"]["code"]=="NOT_FOUND"
            uuid.UUID(headers["x-request-id"])
        status,body,_=call(4190,"/auth/customer/logout","POST",b"x"*65537)
        assert status==413 and json.loads(body)["error"]["code"]=="PAYLOAD_TOO_LARGE"
        status,body,_=call(4190,"/auth/customer/callback")
        assert status==400 and json.loads(body)["error"]["code"]=="INVALID_REQUEST"
        callback="/api/v1/providers/simulated/controls"
        packet={"timestamp":int(time.time()),"signature":"forged","body":{}}
        assert call(4190,callback,"POST",json.dumps(packet).encode(),{"Content-Type":"application/json"})[0]==401
        resource=str(uuid.uuid4())
        check=json.loads(subprocess.check_output([str(ROOT/"target/debug/bank-control-check"),resource,"IDENTITY"],cwd=ROOT,text=True))
        decision={"request":check,"outcome":"ALLOW","evidenceId":str(uuid.uuid4()),"expiresAt":(datetime.now(timezone.utc)+timedelta(minutes=5)).isoformat(),"reasonCode":"SIMULATED_DECISION"}
        timestamp=int(time.time())
        digest=hashlib.sha256(json.dumps(decision,sort_keys=True,separators=(",",":"),ensure_ascii=False).encode()).hexdigest()
        signature=base64.urlsafe_b64encode(hmac.new(b"synthetic-provider-authentication-key-only-2026",f"{timestamp}\n{digest}".encode(),hashlib.sha256).digest()).decode().rstrip("=")
        packet={"timestamp":timestamp,"signature":signature,"body":decision}
        for _ in range(2):
            assert call(4190,callback,"POST",json.dumps(packet).encode(),{"Content-Type":"application/json"})[0]==204
        # Correctly authenticated conflicting evidence is retained and cannot overwrite the first result.
        decision["outcome"]="DENY"
        digest=hashlib.sha256(json.dumps(decision,sort_keys=True,separators=(",",":"),ensure_ascii=False).encode()).hexdigest()
        packet["signature"]=base64.urlsafe_b64encode(hmac.new(b"synthetic-provider-authentication-key-only-2026",f"{timestamp}\n{digest}".encode(),hashlib.sha256).digest()).decode().rstrip("=")
        assert call(4190,callback,"POST",json.dumps(packet).encode(),{"Content-Type":"application/json"})[0]==409
        # Keep this bounded fixed-window assertion away from a minute boundary.
        if time.time()%60>55:time.sleep(61-time.time()%60)
        scope="ingress-test-"+str(uuid.uuid4())
        processes.extend([start(4191,3,scope),start(4192,3,scope)])
        assert call(4191,"/openapi.json")[0]==200
        assert call(4192,"/not-found")[0]==404
        status,body,headers=call(4191,"/openapi.json","POST",b"")
        assert status==405 and "GET" in headers["Allow"]
        assert call(4192,"/openapi.json")[0]==429
        assert call(4191,"/openapi.json",headers={"X-Forwarded-For":"203.0.113.50"})[0]==429
        assert call(4191,"/health/live")[0]==200
        print(json.dumps({"result":"passed","implemented_paths":len(spec["paths"]),"safe_errors":True,"size_limit":True,"shared_two_instance_quota":True,"forwarded_address_spoof_rejected":True}))
    finally:
        for process in processes:process.terminate()
        for process in processes:process.wait(timeout=10)

if __name__=="__main__":main()
