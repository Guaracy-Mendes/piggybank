"""Exercise the operator's file packing boundary without provider or bank access."""
from datetime import datetime, timezone, timedelta
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import uuid
ROOT=Path(__file__).resolve().parents[2]
class PackingChecks(unittest.TestCase):
    def row(self):return {'lineId':str(uuid.uuid4()),'paymentId':str(uuid.uuid4()),'evidenceId':str(uuid.uuid4()),'providerVersion':1,'principalMinor':'9007199254740993','providerFeeMinor':'0','currencyCode':'TST','currencyScale':2,'businessDate':datetime.now(timezone.utc).date().isoformat()}
    def invoke(self,root,rows,kind="outgoing"):
        source=root/'rows.json';source.write_text(json.dumps(rows));now=datetime.now(timezone.utc)
        return subprocess.run(['python3',str(ROOT/'scripts/prepare_settlement_set.py'),str(source),str(root/'packed'),'--kind',kind,'--from-date',now.date().isoformat(),'--to-date',(now+timedelta(days=1)).date().isoformat(),'--cutoff',now.isoformat()],env={**os.environ,'BANK_PROFILE':'local'},capture_output=True,text=True,timeout=10)
    def test_preserves_large_exact_strings_duplicates_order_common_cutoff_and_private_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);row=self.row();rows=[row]*205;result=self.invoke(root,rows);self.assertEqual(result.returncode,0,result.stderr)
            folder=root/'packed';files=[json.loads(p.read_text()) for p in sorted(folder.glob('file-*.json'))];self.assertEqual([len(f['rows']) for f in files],[100,100,5]);self.assertEqual([r for f in files for r in f['rows']],rows)
            manifest=json.loads((folder/'settlement-set.json').read_text());self.assertEqual(manifest['fileIds'],[f['fileId'] for f in files]);self.assertTrue(all(f['generatedAt']==manifest['generatedAt'] for f in files));self.assertTrue(all(f['range']==manifest['range'] for f in files));self.assertEqual(folder.stat().st_mode&0o777,0o700)
            for p in folder.iterdir():self.assertEqual(p.stat().st_mode&0o777,0o600);self.assertLessEqual(p.stat().st_size,60000)
    def test_bounds_and_numeric_input_fail_before_creating_partial_output(self):
        for rows in [[self.row()]*1001,[{**self.row(),'principalMinor':9007199254740993}],[{**self.row(),'providerFeeMinor':'01'}],[{**self.row(),'unknown':'x'*60000}]]:
            with self.subTest(rows=len(rows)),tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);result=self.invoke(root,rows);self.assertNotEqual(result.returncode,0);self.assertFalse((root/'packed').exists())
    def test_existing_output_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);folder=root/'packed';folder.mkdir();(folder/'original').write_text('retain')
            result=self.invoke(root,[]);self.assertNotEqual(result.returncode,0);self.assertEqual((folder/'original').read_text(),'retain')
    def test_credit_namespace_preserves_original_linkage_and_amounts(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);row=self.row();movement=row.pop('paymentId');row.update({'movementId':movement,'eventType':'RETURN_SETTLED','originalPaymentId':str(uuid.uuid4()),'receivingReference':None})
            result=self.invoke(root,[row],"credits");self.assertEqual(result.returncode,0,result.stderr)
            manifest=json.loads((root/'packed/settlement-set.json').read_text());self.assertEqual(manifest['kind'],'CREDITS');self.assertEqual(json.loads((root/'packed/file-001.json').read_text())['rows'],[row])
    def test_empty_provider_snapshot_still_has_one_explicit_member(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);result=self.invoke(root,[]);self.assertEqual(result.returncode,0,result.stderr);manifest=json.loads((root/'packed/settlement-set.json').read_text());self.assertEqual(len(manifest['fileIds']),1);self.assertEqual(json.loads((root/'packed/file-001.json').read_text())['rows'],[])
if __name__=='__main__':unittest.main()
