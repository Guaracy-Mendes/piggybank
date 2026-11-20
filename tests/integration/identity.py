#!/usr/bin/env python3
"""Real local issuer/BFF acceptance branches. Credentials/tokens never print."""
import base64
from database_fixture import Database
import copy
import http.cookiejar
from html.parser import HTMLParser
import importlib.util
import json
import hashlib
import secrets
from pathlib import Path
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location("fixtures",ROOT/"scripts/identity.py")
fixtures=importlib.util.module_from_spec(spec);spec.loader.exec_module(fixtures)
BASE="http://127.0.0.1:4180"
TEST_DATABASE=None

class Form(HTMLParser):
    def __init__(self):super().__init__();self.action=None
    def handle_starttag(self,tag,attrs):
        values=dict(attrs)
        if tag=="form" and values.get("id")=="kc-form-login":self.action=values["action"]

class LoopbackCookiePolicy(http.cookiejar.DefaultCookiePolicy):
    # Browsers treat loopback as a trustworthy local context. Python's cookie
    # jar does not; emulate only that exception in this synthetic HTTP harness.
    # Application/server cookie settings are not weakened by the test.
    def return_ok_secure(self,cookie,request):
        if urllib.parse.urlsplit(request.full_url).hostname=="127.0.0.1" and request.type=="http":return True
        return super().return_ok_secure(cookie,request)

def call(opener,path,method="GET",body=None,headers=None):
    req=urllib.request.Request(path if path.startswith("http") else BASE+path,data=body,method=method,headers=headers or {})
    try:response=opener.open(req,timeout=10)
    except urllib.error.HTTPError as error:response=error
    with response:
        response.headers["X-Test-Final-Path"]=urllib.parse.urlsplit(response.geturl()).path
        return response.status,response.read(),response.headers

def browser_login(trust,username):
    jar=http.cookiejar.CookieJar(policy=LoopbackCookiePolicy());opener=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
    status,body,_=call(opener,f"/auth/{trust}/login")
    assert status==200,status
    form=Form();form.feed(body.decode());assert form.action,"Expected local issuer login form"
    encoded=urllib.parse.urlencode({"username":username,"password":"Synthetic-Local-123!","credentialId":""}).encode()
    status,body,final_headers=call(opener,form.action,"POST",encoded,{"Content-Type":"application/x-www-form-urlencoded"})
    assert status==200,("Login failed",status,final_headers.get("X-Test-Final-Path"),[(c.name,c.domain,c.path,c.secure) for c in jar])
    identity=json.loads(body)
    assert identity["actor"]["trust"]==trust.upper()
    session=next(c for c in jar if c.name=="pb_"+trust)
    assert "HttpOnly" in session._rest
    assert identity["expiresAt"]-time.time()<=32
    return opener,jar,identity

def sql(statement):
    if TEST_DATABASE is None:raise RuntimeError("Explicit HTTP database fixture required")
    return TEST_DATABASE.sql(statement)

def stored_token(statement):
    """Server-held tokens are sealed (AES-256-GCM, row-bound). The harness opens them with the documented
    synthetic local key only to prove the plaintext is a valid issuer token; the database never holds it."""
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    row=sql(statement.replace("SELECT access_token","SELECT id_hash||'|'||access_token"))
    id_hash,sealed=row.split("|",1)
    assert sealed.startswith("v1:synthetic-v1:") and "." not in sealed.split(":",2)[2][:40],"session tokens must be sealed at rest"
    raw=base64.b64decode(sealed.split(":",2)[2])
    key=hashlib.sha256(b"piggybank-synthetic-local-session-key").digest()
    return AESGCM(key).decrypt(raw[:12],raw[12:],f"{id_hash}:access".encode()).decode()

def step_login(opener,challenge,username,expected):
    status,body,_=call(opener,"/auth/customer/step-up/"+challenge)
    assert status==200
    form=Form();form.feed(body.decode());assert form.action
    data=urllib.parse.urlencode({"username":username,"password":"Synthetic-Local-123!","credentialId":""}).encode()
    status,body,_=call(opener,form.action,"POST",data,{"Content-Type":"application/x-www-form-urlencoded"})
    assert status==expected,("Action-bound reauthentication failed",status)
    return json.loads(body)

def main():
    global TEST_DATABASE
    TEST_DATABASE=Database()
    process=subprocess.Popen([str(ROOT/"target/debug/bank-api")],cwd=ROOT,env=TEST_DATABASE.env,stdout=subprocess.DEVNULL)
    admin=None;alice_id=None;rotation_id=None;challenge=None
    try:
        for _ in range(40):
            try:
                if call(urllib.request.build_opener(),"/health/ready")[0]==200:break
            except OSError:pass
            time.sleep(.25)
        else:raise AssertionError("API readiness timeout")
        # UI-02: registration starts the same PKCE login at the issuer with prompt=create; customers only.
        class NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self,*args,**kwargs):return None
        status,_,headers=call(urllib.request.build_opener(NoRedirect),"/auth/customer/register")
        location=urllib.parse.parse_qs(urllib.parse.urlsplit(headers["Location"]).query)
        assert status in (302,303) and location["prompt"]==["create"] and location["code_challenge_method"]==["S256"] and location["client_id"]==["customer-web"],(status,location)
        assert "pb_login_customer=" in headers.get("Set-Cookie","")
        assert call(urllib.request.build_opener(NoRedirect),"/auth/workforce/register")[0]==404
        assert call(urllib.request.build_opener(NoRedirect),"/auth/service/register")[0]==404
        alice,jar,identity=browser_login("customer","alice")
        staff,staff_jar,staff_identity=browser_login("workforce","reviewer")
        # UI-03: contact verification is customer-only, reads the issuer's email, and needs CSRF to send.
        status,body,_=call(alice,"/auth/customer/contact-verification")
        contact=json.loads(body)
        assert status==200 and contact["verified"] is True and contact["destination"]=="a•••@example.invalid" and contact["delivery"]=="SIMULATED_OUTBOX",contact
        assert call(alice,"/auth/customer/contact-verification","POST",b"")[0]==403
        alice_csrf=next(c.value for c in jar if c.name=="pb_csrf_customer")
        assert call(alice,"/auth/customer/contact-verification","POST",b"",{"X-CSRF-Token":alice_csrf,"Origin":BASE})[0]==409
        assert call(staff,"/auth/workforce/contact-verification")[0]==404
        assert call(urllib.request.build_opener(),"/auth/customer/contact-verification")[0]==401
        assert call(alice,"/auth/workforce/me")[0]==401
        assert call(staff,"/auth/customer/me")[0]==401
        assert call(staff,"/auth/workforce/me",headers={"Origin":"https://untrusted.invalid"})[0]==403
        service_body=urllib.parse.urlencode({"grant_type":"client_credentials","client_id":"bank-worker","client_secret":"synthetic-service-client-only"}).encode()
        with urllib.request.urlopen(fixtures.BASE+"/realms/workforce/protocol/openid-connect/token",service_body,timeout=10) as response:service_token=json.load(response)["access_token"]
        assert call(urllib.request.build_opener(),"/auth/service/me",headers={"Authorization":"Bearer "+service_token})[0]==200
        assert call(urllib.request.build_opener(),"/auth/workforce/me",headers={"Authorization":"Bearer "+service_token})[0]==401
        grant=urllib.parse.urlencode({"grant_type":"password","client_id":"customer-web","client_secret":"synthetic-customer-client-only","username":"alice","password":"Synthetic-Local-123!"}).encode()
        grant_status,grant_body,_=call(urllib.request.build_opener(),fixtures.BASE+"/realms/customers/protocol/openid-connect/token","POST",grant)
        assert grant_status==400 and json.loads(grant_body)["error"]=="unauthorized_client"
        alice_id=identity["actor"]["subject"]
        # Only synthetic, validated UUID subjects are interpolated by this test admin harness.
        import uuid
        alice_id=str(uuid.UUID(alice_id))
        token=stored_token(f"SELECT access_token FROM identity.sessions WHERE subject='{alice_id}' ORDER BY created_at DESC LIMIT 1")
        assert call(urllib.request.build_opener(),"/auth/customer/me",headers={"Authorization":"Bearer "+token})[0]==200
        assert call(urllib.request.build_opener(),"/auth/workforce/me",headers={"Authorization":"Bearer "+token})[0]==401
        unsigned=base64.urlsafe_b64encode(json.dumps({"alg":"none"}).encode()).decode().rstrip("=")+"."+token.split(".")[1]+"."
        assert call(urllib.request.build_opener(),"/auth/customer/me",headers={"Authorization":"Bearer "+unsigned})[0]==401
        assert call(alice,"/auth/customer/callback?state=wrong&code=wrong")[0]==401
        assert call(alice,"/auth/customer/logout","POST",b"")[0]==403
        csrf=next(c.value for c in jar if c.name=="pb_csrf_customer")
        assert call(alice,"/auth/customer/logout","POST",b"",{"X-CSRF-Token":csrf,"Origin":"https://untrusted.invalid"})[0]==403
        # Force a server-held expired/invalid access token to exercise real refresh, without leaking it.
        sql(f"UPDATE identity.sessions SET access_token='expired-test-token' WHERE subject='{alice_id}'")
        assert call(alice,"/auth/customer/me")[0]==200
        admin=fixtures.admin_token()
        fixtures.call(admin,"customers/users/"+alice_id,"PUT",{"enabled":False})
        fixtures.call(admin,"customers/users/"+alice_id+"/logout","POST")
        sql(f"UPDATE identity.sessions SET access_token='expired-test-token' WHERE subject='{alice_id}'")
        assert call(alice,"/auth/customer/me")[0]==401
        fixtures.call(admin,"customers/users/"+alice_id,"PUT",{"enabled":True})
        alice,jar,_=browser_login("customer","alice")
        person=str(uuid.uuid4());challenge=str(uuid.uuid4());operation=str(uuid.uuid4());nonce=secrets.token_hex(32)
        digest=hashlib.sha256(b"synthetic action-bound fixture").hexdigest()
        sql(f"INSERT INTO controls.person_links(issuer,subject,person_id) VALUES('{fixtures.BASE}/realms/customers','{alice_id}','{person}') ON CONFLICT DO NOTHING")
        person=sql(f"SELECT person_id FROM controls.person_links WHERE issuer='{fixtures.BASE}/realms/customers' AND subject='{alice_id}'")
        sql(f"INSERT INTO controls.approval_challenges(id,operation_id,issuer,subject,person_id,intent_digest,nonce,state,expires_at) VALUES('{challenge}','{operation}','{fixtures.BASE}/realms/customers','{alice_id}','{person}','{digest}','{nonce}','PENDING',now()+interval '3 minutes')")
        # Keep Alice's BFF session but remove issuer SSO cookies so the issuer
        # actually authenticates Bob, rather than reauthenticating its fixed SSO user.
        wrong_jar=http.cookiejar.CookieJar(policy=LoopbackCookiePolicy())
        for item in jar:
            if item.name.startswith("pb_"):wrong_jar.set_cookie(copy.copy(item))
        wrong=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(wrong_jar))
        step_login(wrong,challenge,"bob",401)
        assert sql(f"SELECT count(*) FROM identity.step_up_proofs WHERE challenge_id='{challenge}'")=="0"
        step_login(alice,challenge,"alice",200)
        assert sql(f"SELECT count(*) FROM identity.step_up_proofs WHERE challenge_id='{challenge}' AND subject='{alice_id}' AND intent_digest='{digest}' AND use_operation_id IS NULL")=="1"
        sql(f"UPDATE controls.approval_challenges SET expires_at=now()-interval '1 second' WHERE id='{challenge}'")
        assert call(alice,"/auth/customer/step-up/"+challenge)[0]==404
        csrf=next(c.value for c in jar if c.name=="pb_csrf_customer")
        status,body,_=call(alice,"/auth/customer/logout","POST",b"",{"X-CSRF-Token":csrf,"Origin":BASE})
        assert status==200 and json.loads(body)["status"]=="signed_out"
        assert call(alice,"/auth/customer/me")[0]==401
        realm=fixtures.call(admin,"customers")
        name="wp04-rotation-"+str(uuid.uuid4())
        fixtures.call(admin,"customers/components","POST",{"name":name,"parentId":realm["id"],"providerId":"rsa-generated","providerType":"org.keycloak.keys.KeyProvider","config":{"priority":["1000"],"enabled":["true"],"active":["true"],"algorithm":["RS256"],"keySize":["2048"]}})
        rotation_id=next(c["id"] for c in fixtures.call(admin,"customers/components") if c["name"]==name)
        time.sleep(2.1)  # Bound the configured unknown-kid refresh cooldown.
        rotated,_,_=browser_login("customer","alice")
        rotated_token=stored_token(f"SELECT access_token FROM identity.sessions WHERE subject='{alice_id}' ORDER BY created_at DESC LIMIT 1")
        def kid(value):return json.loads(base64.urlsafe_b64decode(value.split('.')[0]+'==='))["kid"]
        assert kid(token)!=kid(rotated_token)
        assert call(rotated,"/auth/customer/me")[0]==200
        assert "access_token" not in json.dumps(identity) and "refresh_token" not in json.dumps(identity)
        print(json.dumps({"result":"passed","real_oidc_code_pkce":True,"workforce_customer_isolation":True,"csrf_and_unsigned_token_rejected":True,"refresh_and_revocation":True,"logout":True,"issuer_signing_key_rotation":True,"action_bound_reauthentication":True,"tokens_sealed_at_rest":True}))
    finally:
        if admin and alice_id:
            fixtures.call(admin,"customers/users/"+alice_id,"PUT",{"enabled":True})
        if admin and rotation_id:fixtures.call(admin,"customers/components/"+rotation_id,"DELETE")
        if challenge:
            sql(f"DELETE FROM identity.step_up_proofs WHERE challenge_id='{challenge}'; DELETE FROM controls.approval_challenges WHERE id='{challenge}'")
        process.terminate();process.wait(timeout=10)
        TEST_DATABASE.close()

if __name__=="__main__":main()
