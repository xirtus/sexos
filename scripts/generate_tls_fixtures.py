#!/usr/bin/env python3
"""Reproduce PUBLIC controlled TLS lab material, never production credentials.

Host-only fixture tool: cryptography==46.0.3. Keys are deliberately public small
scalars. Profile1 is restricted to probe.sexos.test and a controlled BOOT policy;
it is not a public-Internet CA or a user trust store. Normal builds use checked-in
Rust bytes and do not execute this script or require host crypto libraries.
"""
from pathlib import Path
import datetime,hashlib,json
from cryptography import x509
from cryptography.hazmat.primitives import hashes,serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID,ExtendedKeyUsageOID

ROOT=Path(__file__).resolve().parents[1]
UTC=datetime.timezone.utc
def date(year):return datetime.datetime(year,1,1,tzinfo=UTC)
def key(n):return ec.derive_private_key(n,ec.SECP256R1())
def name(s):return x509.Name([x509.NameAttribute(NameOID.COMMON_NAME,s)])
def certificate(subject,issuer,public,signer,serial,*,ca=False,host=None,begin=2020,end=2099,client=False,server_eku=False):
 b=x509.CertificateBuilder().subject_name(subject).issuer_name(issuer).public_key(public).serial_number(serial).not_valid_before(date(begin)).not_valid_after(date(end))
 b=b.add_extension(x509.BasicConstraints(ca=ca,path_length=1 if ca else None),True)
 b=b.add_extension(x509.KeyUsage(digital_signature=True,content_commitment=False,key_encipherment=False,data_encipherment=False,key_agreement=False,key_cert_sign=ca,crl_sign=ca,encipher_only=False,decipher_only=False),True)
 if host:
  b=b.add_extension(x509.SubjectAlternativeName([x509.DNSName(host)]),False)
  b=b.add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.CLIENT_AUTH if client else ExtendedKeyUsageOID.SERVER_AUTH]),False)
 elif server_eku:
  b=b.add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH]),False)
 return b.sign(signer,hashes.SHA256(),ecdsa_deterministic=True)
def der(cert):return cert.public_bytes(serialization.Encoding.DER)
def array(symbol,data):
 rows=['    '+','.join('0x%02x'%n for n in data[at:at+16])+','for at in range(0,len(data),16)]
 return 'pub const '+symbol+': &[u8] = &[\n'+'\n'.join(rows)+'\n];\n'
def main():
 ca,leaf,other,inter=map(key,[1,2,3,5]);issuer=name('SexOS PUBLIC controlled TLS lab root')
 root=certificate(issuer,issuer,ca.public_key(),ca,1,ca=True)
 foreign_name=name('SexOS PUBLIC unrelated test root')
 other_root=certificate(foreign_name,foreign_name,other.public_key(),other,3,ca=True)
 materials={}
 for symbol,host,begin,end,client,signer,owner in [
  ('LEAF_DER','probe.sexos.test',2020,2099,False,ca,issuer),
  ('WRONG_NAME_DER','wrong.sexos.test',2020,2099,False,ca,issuer),
  ('EXPIRED_DER','probe.sexos.test',2020,2021,False,ca,issuer),
  ('FUTURE_DER','probe.sexos.test',2090,2099,False,ca,issuer),
  ('CLIENT_ONLY_DER','probe.sexos.test',2020,2099,True,ca,issuer),
  ('FOREIGN_DER','probe.sexos.test',2020,2099,False,other,foreign_name)]:
  materials[symbol]=der(certificate(name(host),owner,leaf.public_key(),signer,100+len(materials),host=host,begin=begin,end=end,client=client))
 inter_name=name('SexOS PUBLIC controlled intermediate')
 materials['INTERMEDIATE_DER']=der(certificate(inter_name,issuer,inter.public_key(),ca,200,ca=True,server_eku=True))
 materials['CHAIN_LEAF_DER']=der(certificate(name('probe.sexos.test'),inter_name,leaf.public_key(),inter,201,host='probe.sexos.test'))
 materials['OTHER_ROOT_DER']=der(other_root)
 transcript=b'SexOS controlled certificate trust self-check v1'
 digest=hashlib.sha256(transcript).digest()
 message=b' '*64+b'TLS 1.3, server CertificateVerify\0'+digest
 materials['TRANSCRIPT']=transcript
 materials['CERTIFICATE_VERIFY_DER']=leaf.sign(message,ec.ECDSA(hashes.SHA256(),deterministic_signing=True))
 config=ROOT/'crates/sex-trust-config/src';config.mkdir(parents=True,exist_ok=True)
 (config/'lib.rs').write_text('''// SPDX-License-Identifier: GPL-3.0-only
//! Immutable BOOT policy data for a PUBLIC controlled TLS lab profile.
//! These public bytes are not capabilities, endpoint grants or a trust store.
//! Only an exact held-source/consumer attestation sanctions their use by an app.
#![no_std]
pub const PROFILE:u32=1;
pub const VERSION:u32=1;
pub const EXPECTED_NAME:&str="probe.sexos.test";
pub const SCOPE:&str="PUBLIC controlled TLS lab root; no public Internet trust";
'''+array('ROOT_DER',der(root)))
 fixtures=ROOT/'crates/sex-tls/src/probe_fixture.rs'
 fixtures.write_text('// SPDX-License-Identifier: GPL-3.0-only\n//! PUBLIC source-bound certificate fixtures. No peer authentication evidence.\n'+''.join(('' if symbol in ('LEAF_DER','TRANSCRIPT','CERTIFICATE_VERIFY_DER') else '#[cfg(test)]\n')+array(symbol,data)for symbol,data in materials.items()))
 directory=ROOT/'tests/fixtures/tls-controlled';directory.mkdir(parents=True,exist_ok=True)
 for label,k in [('root',ca),('server',leaf)]:
  (directory/(label+'-PUBLIC-TEST-key.pem')).write_bytes(k.private_bytes(serialization.Encoding.PEM,serialization.PrivateFormat.PKCS8,serialization.NoEncryption()))
 (directory/'root.pem').write_bytes(root.public_bytes(serialization.Encoding.PEM))
 (directory/'server.pem').write_bytes(x509.load_der_x509_certificate(materials['LEAF_DER']).public_bytes(serialization.Encoding.PEM))
 (directory/'README.md').write_text('PUBLIC TEST KEYS: root scalar1/server scalar2, P256. These are deliberately public controlled lab fixtures, not credentials. Only probe.sexos.test/profile1 is sanctioned by BOOT. No public Internet trust or production security claim. Reproduce with scripts/generate_tls_fixtures.py using cryptography==46.0.3.\n')
 print(json.dumps(dict(root_bytes=len(der(root)),expected_name='probe.sexos.test',public_test_keys=True,fixtures={s:len(d)for s,d in materials.items()})))
if __name__=='__main__':main()
