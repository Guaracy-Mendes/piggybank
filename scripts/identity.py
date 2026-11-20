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

def admin_client(token):
    """WP-28 recovery executor: least-privilege service client (manage/view users in the customers realm only)."""
    secret=os.environ.get("BANK_IDENTITY_ADMIN_SECRET","synthetic-identity-admin-only")
    secret_file=os.environ.get("BANK_IDENTITY_ADMIN_SECRET_FILE")
    if secret_file and Path(secret_file).exists():secret=Path(secret_file).read_text().strip()
    payload={"clientId":"bank-identity-admin","enabled":True,"protocol":"openid-connect","publicClient":False,"standardFlowEnabled":False,"directAccessGrantsEnabled":False,
             "serviceAccountsEnabled":True,"secret":secret,"attributes":{}}
    existing=call(token,"customers/clients?clientId=bank-identity-admin")
    if existing:
        payload["id"]=existing[0]["id"];call(token,"customers/clients/"+payload["id"],"PUT",payload)
    else:
        call(token,"customers/clients","POST",payload);existing=call(token,"customers/clients?clientId=bank-identity-admin");payload["id"]=existing[0]["id"]
    management=call(token,"customers/clients?clientId=realm-management")[0]["id"]
    wanted=[call(token,f"customers/clients/{management}/roles/{name}") for name in ["manage-users","view-users"]]
    service_user=call(token,f"customers/clients/{payload['id']}/service-account-user")["id"]
    call(token,f"customers/users/{service_user}/role-mappings/clients/{management}","POST",[{"id":r["id"],"name":r["name"]} for r in wanted])

def registration_enabled():
    """UI-02 self-service registration: on for local synthetic issuers unless BANK_CUSTOMER_REGISTRATION=disabled
    (the cloud template disables it). Email ownership is not verified until an OTP provider exists (UI-03)."""
    value=os.environ.get("BANK_CUSTOMER_REGISTRATION","enabled")
    if value not in {"enabled","disabled"}:raise SystemExit("BANK_CUSTOMER_REGISTRATION must be enabled or disabled")
    return value=="enabled"

def registration(token):
    """New customers get the same realm role as fixtures and must tick the synthetic-demo terms on the registration
    form (the registration flow's terms step). Existing users are never re-prompted."""
    for step in call(token,"customers/authentication/flows/registration/executions"):
        if step["providerId"]=="registration-terms-and-conditions" and step["requirement"]!="REQUIRED":
            step["requirement"]="REQUIRED";call(token,"customers/authentication/flows/registration/executions","PUT",step)
    customer=call(token,"customers/roles/customer")
    defaults=call(token,"customers/roles/default-roles-customers/composites/realm")
    if not any(r["name"]=="customer" for r in defaults):
        call(token,"customers/roles/default-roles-customers/composites","POST",[{"id":customer["id"],"name":"customer"}])

PARTNER_CLIENTS=["partner-budget","partner-other"]

def partner_realm(token):
    """WP-33: a separate realm for partner applications (client credentials only, 30 s tokens, bank-partner audience).
    Registration in the bank's own registry (staff maker/checker) is still required before any token is accepted."""
    try:
        call(token,"partners")
    except urllib.error.HTTPError as error:
        if error.code!=404: raise
        request=urllib.request.Request(BASE+"/admin/realms",data=json.dumps({"realm":"partners","enabled":True}).encode(),method="POST",headers={"Authorization":"Bearer "+token,"Content-Type":"application/json"})
        urllib.request.urlopen(request,timeout=10).close()
    call(token,"partners","PUT",{"realm":"partners","enabled":True,"accessTokenLifespan":30,"registrationAllowed":False,"eventsEnabled":True,"adminEventsEnabled":True,"adminEventsDetailsEnabled":False})
    for name in PARTNER_CLIENTS:
        secret=os.environ.get("BANK_"+name.upper().replace("-","_")+"_SECRET",f"synthetic-{name}-only")
        payload={"clientId":name,"enabled":True,"protocol":"openid-connect","publicClient":False,"standardFlowEnabled":False,"directAccessGrantsEnabled":False,
                 "implicitFlowEnabled":False,"serviceAccountsEnabled":True,"secret":secret,"attributes":{},
                 "protocolMappers":[{"name":"bank-audience","protocol":"openid-connect","protocolMapper":"oidc-audience-mapper","consentRequired":False,
                                     "config":{"included.custom.audience":"bank-partner","access.token.claim":"true","id.token.claim":"false"}}]}
        existing=call(token,"partners/clients?clientId="+name)
        if existing:
            payload["id"]=existing[0]["id"];call(token,"partners/clients/"+payload["id"],"PUT",payload)
        else:
            call(token,"partners/clients","POST",payload)

def provision():
    token=admin_token()
    for realm,trust,users in [("customers","customer",["alice","bob"]),("workforce","workforce",["reviewer","approver","security"] )]:
        config=json.loads((Path(os.environ.get("BANK_REALM_DIR",str(ROOT/"infra/identity")))/f"{realm}-realm.json").read_text())
        config.update({"accessTokenLifespan":30,"revokeRefreshToken":True,"refreshTokenMaxReuse":0,"eventsEnabled":True,"adminEventsEnabled":True,"adminEventsDetailsEnabled":False})
        if realm=="customers":config["registrationAllowed"]=registration_enabled()
        call(token,realm,"PUT",config)
        clients=[client("customer-web","customer"),client("bank-mobile","customer",True)] if realm=="customers" else [client("staff-web","workforce"),client("bank-worker","service",service=True)]
        for payload in clients:
            existing=call(token,realm+"/clients?clientId="+payload["clientId"])
            if existing:
                payload["id"]=existing[0]["id"]
                call(token,realm+"/clients/"+payload["id"],"PUT",payload)
            else:
                call(token,realm+"/clients","POST",payload)
        if realm=="customers":admin_client(token)
        roles=["customer"] if realm=="customers" else ["reviewer","approver","security"]
        existing_roles={r["name"] for r in call(token,realm+"/roles")}
        for role in roles:
            if role not in existing_roles: call(token,realm+"/roles","POST",{"name":role})
        if realm=="customers":registration(token)
        for username in users:
            found=call(token,realm+"/users?exact=true&username="+username)
            if not found:
                call(token,realm+"/users","POST",{"username":username,"enabled":True,"firstName":username.title(),"lastName":"Synthetic","email":username+"@example.invalid","emailVerified":True,"credentials":[{"type":"password","value":os.environ.get("BANK_FIXTURE_PASSWORD","Synthetic-Local-123!"),"temporary":False}]})
                found=call(token,realm+"/users?exact=true&username="+username)
            role=call(token,realm+"/roles/"+("customer" if realm=="customers" else username))
            call(token,realm+"/users/"+found[0]["id"]+"/role-mappings/realm","POST",[role])
    partner_realm(token)
    print("Synthetic issuer clients, roles and users configured; password grants disabled for application clients.")

if __name__=="__main__":provision()
