#!/usr/bin/env python3
"""Idempotent local-only Keycloak fixtures; never prints credentials or tokens."""
import json
import os
from pathlib import Path
import urllib.error
import urllib.parse
import urllib.request

BASE=os.environ.get("BANK_IDENTITY_ADMIN_BASE","http://127.0.0.1:18280")
ROOT=Path(__file__).resolve().parents[1]

def admin_token():
    body=urllib.parse.urlencode({"grant_type":"password","client_id":"admin-cli","username":"local-admin","password":os.environ.get("KEYCLOAK_ADMIN_PASSWORD","synthetic-keycloak-admin-only")}).encode()
    with urllib.request.urlopen(BASE+"/realms/master/protocol/openid-connect/token",body,timeout=10) as response:
        return json.load(response)["access_token"]

def call(token,path,method="GET",body=None):
    request=urllib.request.Request(BASE+"/admin/realms/"+path,data=json.dumps(body).encode() if body is not None else None,method=method,headers={"Authorization":"Bearer "+token,"Content-Type":"application/json"})
    with urllib.request.urlopen(request,timeout=10) as response:
        raw=response.read()
        return json.loads(raw) if raw else None

def client(name,trust,public=False,service=False):
    audience={"customer":"bank-customer","workforce":"bank-workforce","service":"bank-service"}[trust]
    result={"clientId":name,"enabled":True,"protocol":"openid-connect","publicClient":public,"standardFlowEnabled":not service,"directAccessGrantsEnabled":False,"serviceAccountsEnabled":service,"redirectUris":[f"http://127.0.0.1:4180/auth/{trust}/callback"] if not public else ["piggybank://auth/callback"],"webOrigins":["http://127.0.0.1:4180"],"attributes":{"pkce.code.challenge.method":"S256","post.logout.redirect.uris":f"http://127.0.0.1:4180/auth/{trust}/signed-out"},"protocolMappers":[{"name":"bank-audience","protocol":"openid-connect","protocolMapper":"oidc-audience-mapper","consentRequired":False,"config":{"included.custom.audience":audience,"access.token.claim":"true","id.token.claim":"false"}}]}
    if not public:
        result["secret"]=os.environ.get(f"BANK_{trust.upper()}_CLIENT_SECRET",f"synthetic-{trust}-client-only")
    return result

def provision():
    token=admin_token()
    for realm,trust,users in [("customers","customer",["alice","bob"]),("workforce","workforce",["reviewer","approver","security"] )]:
        config=json.loads((Path(os.environ.get("BANK_REALM_DIR",str(ROOT/"infra/identity")))/f"{realm}-realm.json").read_text())
        config.update({"accessTokenLifespan":30,"revokeRefreshToken":True,"refreshTokenMaxReuse":0,"eventsEnabled":True,"adminEventsEnabled":True,"adminEventsDetailsEnabled":False})
        call(token,realm,"PUT",config)
        clients=[client("customer-web","customer"),client("bank-mobile","customer",True)] if realm=="customers" else [client("staff-web","workforce"),client("bank-worker","service",service=True)]
        for payload in clients:
            existing=call(token,realm+"/clients?clientId="+payload["clientId"])
            if existing:
                payload["id"]=existing[0]["id"]
                call(token,realm+"/clients/"+payload["id"],"PUT",payload)
            else:
                call(token,realm+"/clients","POST",payload)
        roles=["customer"] if realm=="customers" else ["reviewer","approver","security"]
        existing_roles={r["name"] for r in call(token,realm+"/roles")}
        for role in roles:
            if role not in existing_roles: call(token,realm+"/roles","POST",{"name":role})
        for username in users:
            found=call(token,realm+"/users?exact=true&username="+username)
            if not found:
                call(token,realm+"/users","POST",{"username":username,"enabled":True,"firstName":username.title(),"lastName":"Synthetic","email":username+"@example.invalid","emailVerified":True,"credentials":[{"type":"password","value":os.environ.get("BANK_FIXTURE_PASSWORD","Synthetic-Local-123!"),"temporary":False}]})
                found=call(token,realm+"/users?exact=true&username="+username)
            role=call(token,realm+"/roles/"+("customer" if realm=="customers" else username))
            call(token,realm+"/users/"+found[0]["id"]+"/role-mappings/realm","POST",[role])
    print("Synthetic issuer clients, roles and users configured; password grants disabled for application clients.")

if __name__=="__main__":provision()
