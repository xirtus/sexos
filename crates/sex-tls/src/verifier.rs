// SPDX-License-Identifier: GPL-3.0-only
//! Limited controlled-profile P256 verification, with no ambient root/name/time.
//! Public policy bytes alone are data: parent must first attest this profile on
//! its exact BOOT-held route. A certificate check is not a completed connection.
use embedded_tls::{Aes128GcmSha256,CertificateEntryRef,CertificateRef,CertificateVerifyRef,
    SignatureScheme,TlsError,TlsVerifier};
use p256::ecdsa::{Signature,VerifyingKey,signature::Verifier as _};
use rustls_pki_types::{AlgorithmIdentifier,CertificateDer,InvalidSignature,ServerName,
    SignatureVerificationAlgorithm,UnixTime,alg_id};
use sha2::{Digest,Sha256};
use sex_trust_config as policy;

pub const MAX_CERT_BYTES:usize=1024;
pub const MAX_CHAIN:usize=3;
const SERVER_AUTH:&[u8]=&[0x2b,6,1,5,5,7,3,1];
#[derive(Debug)]
struct P256Sha256;
impl SignatureVerificationAlgorithm for P256Sha256 {
    fn verify_signature(&self,key:&[u8],message:&[u8],signature:&[u8])->Result<(),InvalidSignature> {
        let key=VerifyingKey::from_sec1_bytes(key).map_err(|_|InvalidSignature)?;
        let signature=Signature::from_der(signature).map_err(|_|InvalidSignature)?;
        // webpki provides an UNHASHED message; this hashes exactly once.
        key.verify(message,&signature).map_err(|_|InvalidSignature)
    }
    fn public_key_alg_id(&self)->AlgorithmIdentifier {alg_id::ECDSA_P256}
    fn signature_alg_id(&self)->AlgorithmIdentifier {alg_id::ECDSA_SHA256}
}
static P256:P256Sha256=P256Sha256;

#[derive(Clone,Copy,PartialEq,Eq,Debug)]
enum Stage { Initial,NameBound,Certificate,Signature,Failed }
pub(crate) struct Verifier {
    seconds:u32,stage:Stage,leaf:[u8;MAX_CERT_BYTES],leaf_len:usize,transcript:[u8;32],
}
impl Verifier {
    pub(crate) fn new(profile:u32,seconds:u32)->Result<Self,TlsError> {
        if profile!=policy::PROFILE||!(1_577_836_800..=4_102_444_799).contains(&seconds) {
            return Err(TlsError::InvalidCertificate);
        }
        Ok(Self{seconds,stage:Stage::Initial,leaf:[0;MAX_CERT_BYTES],leaf_len:0,transcript:[0;32]})
    }
    fn fail(&mut self,error:TlsError)->TlsError {
        self.stage=Stage::Failed;crate::erase(&mut self.leaf);crate::erase(&mut self.transcript);self.leaf_len=0;error
    }
    fn chain(&mut self,cert:CertificateRef)->Result<(),TlsError> {
        if cert.entries.is_empty()||cert.entries.len()>MAX_CHAIN {return Err(TlsError::InvalidCertificate);}
        let mut certificates=[CertificateDer::from(&[][..]),CertificateDer::from(&[][..]),CertificateDer::from(&[][..])];
        for (index,entry)in cert.entries.iter().enumerate() {
            let CertificateEntryRef::X509(bytes)=entry else{return Err(TlsError::InvalidCertificate);};
            if bytes.is_empty()||bytes.len()>MAX_CERT_BYTES{return Err(TlsError::InvalidCertificate);}
            certificates[index]=CertificateDer::from(*bytes);
        }
        let leaf=webpki::EndEntityCert::try_from(&certificates[0]).map_err(|_|TlsError::InvalidCertificate)?;
        let root_der=CertificateDer::from(policy::ROOT_DER);
        let root=webpki::anchor_from_trusted_cert(&root_der).map_err(|_|TlsError::InvalidCertificate)?;
        leaf.verify_for_usage(&[&P256],&[root],&certificates[1..cert.entries.len()],
            UnixTime::since_unix_epoch(core::time::Duration::from_secs(self.seconds as u64)),
            webpki::KeyUsage::required(SERVER_AUTH),None,None).map_err(|_|TlsError::InvalidCertificate)?;
        let name=ServerName::try_from(policy::EXPECTED_NAME).map_err(|_|TlsError::InvalidCertificate)?;
        leaf.verify_is_valid_for_subject_name(&name).map_err(|_|TlsError::InvalidCertificate)?;
        self.leaf_len=certificates[0].as_ref().len();self.leaf[..self.leaf_len].copy_from_slice(certificates[0].as_ref());Ok(())
    }
    /// Upstream open() may accept Finished without both callbacks. This gate is
    /// mandatory after its successful Finished check and cannot be reset.
    pub(crate) fn finished(&mut self)->Result<(),TlsError> {
        if self.stage==Stage::Signature {Ok(())}else{Err(self.fail(TlsError::InvalidCertificate))}
    }
}
impl TlsVerifier<Aes128GcmSha256> for Verifier {
    fn set_hostname_verification(&mut self,name:&str)->Result<(),TlsError> {
        if self.stage!=Stage::Initial||name!=policy::EXPECTED_NAME {return Err(self.fail(TlsError::InvalidCertificate));}
        self.stage=Stage::NameBound;Ok(())
    }
    fn verify_certificate(&mut self,transcript:&Sha256,cert:CertificateRef)->Result<(),TlsError> {
        if self.stage!=Stage::NameBound{return Err(self.fail(TlsError::InvalidCertificate));}
        if let Err(error)=self.chain(cert){return Err(self.fail(error));}
        self.transcript.copy_from_slice(&transcript.clone().finalize());self.stage=Stage::Certificate;Ok(())
    }
    fn verify_signature(&mut self,verify:CertificateVerifyRef)->Result<(),TlsError> {
        if self.stage!=Stage::Certificate||verify.signature_scheme!=SignatureScheme::EcdsaSecp256r1Sha256 {
            return Err(self.fail(TlsError::InvalidSignature));
        }
        let mut message=[b' ';130];message[64..98].copy_from_slice(b"TLS 1.3, server CertificateVerify\0");message[98..].copy_from_slice(&self.transcript);
        let der=CertificateDer::from(&self.leaf[..self.leaf_len]);
        let result=webpki::EndEntityCert::try_from(&der).map_err(|_|TlsError::InvalidCertificate)
            .and_then(|cert|cert.verify_signature(&P256,&message,verify.signature).map_err(|_|TlsError::InvalidSignature));
        crate::erase(&mut message);
        if let Err(error)=result{return Err(self.fail(error));}
        self.stage=Stage::Signature;Ok(())
    }
}
impl Drop for Verifier {fn drop(&mut self){crate::erase(&mut self.leaf);crate::erase(&mut self.transcript);}}

/// Public source-bound fixture, never a peer or wire acceptance claim. Checks
/// the same exact verifier that the owned TLS future will use for real records.
pub fn controlled_self_check(profile:u32,seconds:u32)->Result<(),TlsError> {
    let mut verifier=Verifier::new(profile,seconds)?;
    verifier.set_hostname_verification(policy::EXPECTED_NAME)?;
    let mut transcript=Sha256::new();transcript.update(crate::probe_fixture::TRANSCRIPT);
    let mut cert=CertificateRef::with_context(&[]);cert.add(CertificateEntryRef::X509(crate::probe_fixture::LEAF_DER))?;
    verifier.verify_certificate(&transcript,cert)?;
    verifier.verify_signature(CertificateVerifyRef {signature_scheme:SignatureScheme::EcdsaSecp256r1Sha256,signature:crate::probe_fixture::CERTIFICATE_VERIFY_DER})?;
    verifier.finished()
}

#[cfg(test)]mod tests {
    use super::*;use crate::probe_fixture as f;
    const UTC:u32=1_793_367_907;
    fn bound()->Verifier {let mut v=Verifier::new(policy::PROFILE,UTC).unwrap();v.set_hostname_verification(policy::EXPECTED_NAME).unwrap();v}
    fn certificate<'a>(chain:&[&'a[u8]])->CertificateRef<'a> {let mut c=CertificateRef::with_context(&[]);for bytes in chain {c.add(CertificateEntryRef::X509(bytes)).unwrap();}c}
    fn hash()->Sha256 {let mut h=Sha256::new();h.update(f::TRANSCRIPT);h}
    fn signature()->CertificateVerifyRef<'static> {CertificateVerifyRef {signature_scheme:SignatureScheme::EcdsaSecp256r1Sha256,signature:f::CERTIFICATE_VERIFY_DER}}
    #[test]fn actual_certificate_name_time_and_rfc8446_signature_fixture_passes(){controlled_self_check(policy::PROFILE,UTC).unwrap();}
    #[test]fn absent_wrong_profile_or_clock_never_skips_validation(){for (p,t)in[(0,UTC),(2,UTC),(1,0),(1,u32::MAX)]{assert!(Verifier::new(p,t).is_err());}}
    #[test]fn wrong_name_expired_future_eku_and_foreign_root_fail_permanently(){for bytes in[f::WRONG_NAME_DER,f::EXPIRED_DER,f::FUTURE_DER,f::CLIENT_ONLY_DER,f::FOREIGN_DER]{let mut v=bound();assert!(v.verify_certificate(&hash(),certificate(&[bytes])).is_err());assert!(v.verify_certificate(&hash(),certificate(&[f::LEAF_DER])).is_err());assert!(v.finished().is_err());}}
    #[test]fn intermediate_chain_verifies_and_leaf_is_retained_privately(){let mut v=bound();v.verify_certificate(&hash(),certificate(&[f::CHAIN_LEAF_DER,f::INTERMEDIATE_DER])).unwrap();assert_eq!(&v.leaf[..v.leaf_len],f::CHAIN_LEAF_DER);assert!(v.finished().is_err());}
    #[test]fn every_missing_duplicate_or_out_of_order_callback_prevents_finish(){
        let mut v=bound();assert!(v.finished().is_err());
        let mut v=bound();assert!(v.verify_signature(signature()).is_err());
        let mut v=bound();v.verify_certificate(&hash(),certificate(&[f::LEAF_DER])).unwrap();assert!(v.finished().is_err());
        let mut v=bound();v.verify_certificate(&hash(),certificate(&[f::LEAF_DER])).unwrap();assert!(v.verify_certificate(&hash(),certificate(&[f::LEAF_DER])).is_err());
        let mut v=bound();v.verify_certificate(&hash(),certificate(&[f::LEAF_DER])).unwrap();v.verify_signature(signature()).unwrap();assert!(v.verify_signature(signature()).is_err());assert!(v.finished().is_err());
    }
    #[test]fn wrong_transcript_modified_signature_and_scheme_fail(){
        let mut v=bound();v.verify_certificate(&Sha256::new(),certificate(&[f::LEAF_DER])).unwrap();assert!(v.verify_signature(signature()).is_err());
        let mut v=bound();v.verify_certificate(&hash(),certificate(&[f::LEAF_DER])).unwrap();let mut s=f::CERTIFICATE_VERIFY_DER.to_vec();let n=s.len();s[n-1]^=1;assert!(v.verify_signature(CertificateVerifyRef{signature_scheme:SignatureScheme::EcdsaSecp256r1Sha256,signature:&s}).is_err());
        let mut v=bound();v.verify_certificate(&hash(),certificate(&[f::LEAF_DER])).unwrap();assert!(v.verify_signature(CertificateVerifyRef{signature_scheme:SignatureScheme::Ed25519,signature:f::CERTIFICATE_VERIFY_DER}).is_err());
    }
    #[test]fn empty_raw_key_oversized_and_excess_chains_are_rejected(){
        let mut v=bound();assert!(v.verify_certificate(&hash(),certificate(&[])).is_err());
        let mut v=bound();let mut c=CertificateRef::with_context(&[]);c.add(CertificateEntryRef::RawPublicKey(f::LEAF_DER)).unwrap();assert!(v.verify_certificate(&hash(),c).is_err());
        let mut v=bound();assert!(v.verify_certificate(&hash(),certificate(&[&[0u8;1025]])).is_err());
        let mut v=bound();assert!(v.verify_certificate(&hash(),certificate(&[f::OTHER_ROOT_DER])).is_err());
        let mut v=bound();assert!(v.verify_certificate(&hash(),certificate(&[f::LEAF_DER;4])).is_err());
    }
    #[test]fn hostname_must_be_exact_and_bound_once(){let mut v=Verifier::new(1,UTC).unwrap();assert!(v.set_hostname_verification("wrong.sexos.test").is_err());assert!(v.set_hostname_verification(policy::EXPECTED_NAME).is_err());let mut v=bound();assert!(v.set_hostname_verification(policy::EXPECTED_NAME).is_err());}
}
