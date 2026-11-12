#!/usr/bin/env python3
"""Exercise the fixture publisher's real HTTP boundary without database/provider access."""
import contextlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest

ROOT=Path(__file__).resolve().parents[2]

@contextlib.contextmanager
def endpoint(redirect=None):
    calls=[]
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            calls.append((self.path,self.headers.get('X-Finance-Key'),self.rfile.read(int(self.headers.get('Content-Length','0')))))
            self.send_response(302 if redirect else 201)
            if redirect:self.send_header('Location',redirect)
            self.end_headers()
        do_GET=do_POST
        def log_message(self,*_):pass
    server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
    thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
    try:yield f'http://127.0.0.1:{server.server_port}',calls
    finally:server.shutdown();server.server_close();thread.join()

class PublisherChecks(unittest.TestCase):
    def invoke(self,url,body,kind="outgoing"):
        with tempfile.TemporaryDirectory() as folder:
            file=Path(folder)/'file.json';file.write_bytes(body)
            return subprocess.run(['python3',str(ROOT/'scripts/publish_settlement.py'),str(file),'--kind',kind],env={**os.environ,'BANK_PROFILE':'local','BANK_FINANCE_PROVIDER_BASE':url,'BANK_FINANCE_SIGNING_KEY':'synthetic-transport-test-key-over-thirty-two-bytes'},capture_output=True,text=True,timeout=10)
    def test_exact_string_bytes_and_authentication_are_sent_to_configured_endpoint(self):
        with endpoint() as (url,calls):
            body=b'{"principalMinor":"9007199254740993"}'
            result=self.invoke(url,body)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertEqual(calls,[('/admin/settlement-files','synthetic-transport-test-key-over-thirty-two-bytes',body)])
            self.assertIn('HTTP 201',result.stdout)
    def test_credit_kind_uses_its_distinct_provider_endpoint(self):
        with endpoint() as (url,calls):
            body=b'{"principalMinor":"9007199254740993"}'
            result=self.invoke(url,body,"credits")
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertEqual(calls,[('/admin/credit-settlement-files','synthetic-transport-test-key-over-thirty-two-bytes',body)])
    def test_set_kind_uses_the_closed_manifest_endpoint(self):
        with endpoint() as (url,calls):
            body=b'{"setId":"12345678-1234-1234-1234-123456789abc","fileIds":[]}'
            result=self.invoke(url,body,"set")
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertEqual(calls,[('/admin/settlement-sets','synthetic-transport-test-key-over-thirty-two-bytes',body)])
    def test_redirect_never_forwards_the_provider_key(self):
        with endpoint() as (target,forwarded),endpoint(redirect=target+'/collector') as (url,original):
            result=self.invoke(url,b'{}')
            self.assertNotEqual(result.returncode,0)
            self.assertIn('HTTP 302',result.stderr)
            self.assertEqual(len(original),1)
            self.assertEqual(forwarded,[])
    def test_oversized_input_is_rejected_before_http(self):
        with endpoint() as (url,calls):
            result=self.invoke(url,b' '*60001)
            self.assertNotEqual(result.returncode,0)
            self.assertIn('60,000 byte',result.stderr)
            self.assertEqual(calls,[])
    def test_non_loopback_destination_is_rejected_without_network(self):
        result=self.invoke('http://provider.example.invalid',b'{}')
        self.assertNotEqual(result.returncode,0)
        self.assertIn('loopback',result.stderr)

if __name__=='__main__':unittest.main()
