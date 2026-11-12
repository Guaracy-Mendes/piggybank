#!/usr/bin/env python3
"""Validate deployment credential persistence, channel configuration and generated Compose."""
import hashlib
import io
import tarfile
from artifacts import extract_release, checksum_file
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]
SPEC=importlib.util.spec_from_file_location('bootstrap',ROOT/'infra/cloud/shared/bootstrap.py')
BOOT=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BOOT)

class DeploymentChecks(unittest.TestCase):
    def test_actual_release_archive_rejects_tampering_and_unsafe_members(self):
        for kind in ['valid', 'bad_image', 'bad_outer', 'traversal', 'symlink', 'undeclared', 'duplicate']:
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as folder:
                root=Path(folder);archive=root/'fixture-01.tgz'
                content={'deployment/release.env':b'RELEASE_ID=fixture-01\n','images.tar':b'image archive fixture'}
                manifest={'release':'fixture-01','platform':'linux/amd64','profile':'synthetic-local','schemaVersion':46,
                    'files':{'deployment/release.env':hashlib.sha256(content['deployment/release.env']).hexdigest()},
                    'imagesSha256':hashlib.sha256(content['images.tar']).hexdigest()}
                if kind=='bad_image':content['images.tar']=b'damaged image archive'
                if kind=='undeclared':content['deployment/extra.env']=b'not declared'
                content['manifest.json']=(json.dumps(manifest)+'\n').encode()
                with tarfile.open(archive,'w:gz') as output:
                    for name,data in content.items():
                        info=tarfile.TarInfo(name);info.size=len(data);output.addfile(info,io.BytesIO(data))
                    if kind=='duplicate':
                        info=tarfile.TarInfo('images.tar');info.size=0;output.addfile(info,io.BytesIO(b''))
                    if kind=='traversal':output.addfile(tarfile.TarInfo('../escaped'))
                    if kind=='symlink':
                        info=tarfile.TarInfo('deployment/link');info.type=tarfile.SYMTYPE;info.linkname='/tmp';output.addfile(info)
                digest=checksum_file(archive)
                archive.with_suffix('.tgz.sha256').write_text(f'{digest}  {archive.name}\n')
                if kind=='bad_outer':
                    with archive.open('ab') as stream:stream.write(b'damage')
                destination=root/'extracted';destination.mkdir()
                if kind=='valid':
                    self.assertEqual(extract_release(archive,destination,'fixture-01'),manifest)
                    self.assertEqual((destination/'images.tar').read_bytes(),content['images.tar'])
                else:
                    with self.assertRaises(ValueError):extract_release(archive,destination,'fixture-01')
                self.assertFalse((root/'escaped').exists())
    def test_credentials_are_distinct_persistent_and_private(self):
        with tempfile.TemporaryDirectory() as folder:
            BOOT.STATE=Path(folder)/'state'
            BOOT.create_environment()
            files=[BOOT.STATE/'.env',BOOT.STATE/'runtime.env']
            before=[p.read_bytes() for p in files]
            for p in files:self.assertEqual(p.stat().st_mode & 0o777,0o600)
            control=BOOT.load_env(files[0]);runtime=BOOT.load_env(files[1])
            self.assertNotIn('MIGRATION_DATABASE_URL',runtime)
            self.assertNotIn('POSTGRES_PASSWORD',runtime)
            keys=[v for k,v in runtime.items() if k.endswith('_KEY') or k.endswith('_SECRET')]
            self.assertEqual(len(keys),len(set(keys)))
            self.assertTrue(all(len(k)==64 for k in keys))
            self.assertEqual(runtime['BANK_PROFILE'],'local')
            BOOT.create_environment()
            self.assertEqual(before,[p.read_bytes() for p in files])
            files[1].unlink()
            with self.assertRaises(SystemExit):BOOT.create_environment()
    def test_existing_database_without_original_keys_is_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            BOOT.STATE=Path(folder)
            (BOOT.STATE/'postgres').mkdir()
            (BOOT.STATE/'postgres/PG_VERSION').write_text('18')
            with self.assertRaises(SystemExit):BOOT.create_environment()
    def test_compose_host_boundary_and_migration_privilege(self):
        with tempfile.TemporaryDirectory() as folder:
            BOOT.STATE=Path(folder)/'state';BOOT.create_environment()
            release=Path(folder)/'release.env';release.write_text('RELEASE_ID=validation-01\n')
            config=Path(folder)/'compose.yaml'
            config.write_text((ROOT/'infra/cloud/shared/compose.yaml').read_text().replace('/srv/piggybank/state/runtime.env',str(BOOT.STATE/'runtime.env')))
            result=subprocess.check_output(['docker','compose','--env-file',str(BOOT.STATE/'.env'),
                '--env-file',str(release),'-f',str(config),
                '--profile','tools','config','--no-env-resolution','--format','json'],text=True)
            services=json.loads(result)['services']
            self.assertEqual(len(services),9)
            for config in services.values():
                self.assertEqual(config['network_mode'],'host')
                self.assertFalse(config.get('ports'))
            for name in ['api','worker','provider']:
                self.assertNotIn('MIGRATION_DATABASE_URL',services[name].get('environment',{}))
                self.assertTrue(services[name]['read_only'])
            self.assertIn('MIGRATION_DATABASE_URL',services['migrate']['environment'])
            self.assertIn('127.0.0.1',services['postgres']['command'][-3])
    def test_identity_secret_overrides_preserve_local_defaults(self):
        spec=importlib.util.spec_from_file_location('identity',ROOT/'scripts/identity.py')
        identity=importlib.util.module_from_spec(spec);spec.loader.exec_module(identity)
        from unittest.mock import patch
        with patch.dict(os.environ,{'BANK_CUSTOMER_CLIENT_SECRET':'a'*64}):
            client=identity.client('customer-web','customer')
            self.assertEqual(client['secret'],'a'*64)
            self.assertFalse(client['directAccessGrantsEnabled'])
            self.assertEqual(client['redirectUris'],['http://127.0.0.1:4180/auth/customer/callback'])
    def test_rendered_startup_scripts_parse(self):
        shared=(ROOT/'infra/cloud/shared/host-bootstrap.sh').read_text()
        with tempfile.TemporaryDirectory() as folder:
            for provider in ['gcp','aws']:
                source=(ROOT/f'infra/cloud/{provider}/startup.sh.tftpl').read_text()
                for key,value in {'bootstrap':shared,'bucket':'test-bucket','object':'releases/test.tgz',
                    'release_sha256':'a'*64,'volume_id':'vol12345','region':'us-east-1'}.items():
                    source=source.replace('${'+key+'}',value)
                path=Path(folder)/f'{provider}-startup.sh';path.write_text(source)
                subprocess.run(['bash','-n',str(path)],check=True)

if __name__=='__main__':
    for path in list((ROOT/'infra/cloud').rglob('*.sh')):
        subprocess.run(['bash','-n',str(path)],check=True)
    for path in list((ROOT/'infra/cloud').rglob('*.py'))+[ROOT/'scripts/identity.py',Path(__file__),ROOT/'tests/deployment/runtime.py',ROOT/'tests/deployment/artifacts.py']:
        compile(path.read_text(),str(path),'exec')
    unittest.main()
