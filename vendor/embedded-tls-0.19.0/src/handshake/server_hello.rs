use heapless::Vec;

use crate::cipher_suites::CipherSuite;
use crate::crypto_engine::CryptoEngine;
use crate::extensions::extension_data::key_share::KeyShareEntry;
use crate::extensions::messages::ServerHelloExtension;
use crate::parse_buffer::ParseBuffer;
use crate::TlsError;
use crate::config::TlsCipherSuite;
use crate::extensions::extension_data::{supported_groups::NamedGroup,supported_versions::{TLS13,SupportedVersionsServerHello}};
use crate::extensions::extension_data::key_share::KeyShareServerHello;
use p256::PublicKey;
use p256::ecdh::{EphemeralSecret, SharedSecret};

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ServerHello<'a> {
    extensions: Vec<ServerHelloExtension<'a>, 4>,
}

impl<'a> ServerHello<'a> {
    /// SexOS supports the exact offered suite and a single TLS1.3/P256
    /// exchange. An unoffered selection cannot be inferred from the key bytes.
    pub fn parse<C: TlsCipherSuite>(buf: &mut ParseBuffer<'a>) -> Result<ServerHello<'a>, TlsError> {
        if buf.read_u16()? != 0x0303 { return Err(TlsError::InvalidSupportedVersions); }
        let mut random = [0;32]; buf.fill(&mut random)?;
        const RETRY:[u8;32]=[0xcf,0x21,0xad,0x74,0xe5,0x9a,0x61,0x11,0xbe,0x1d,0x8c,0x02,0x1e,0x65,0xb8,0x91,0xc2,0xa2,0x11,0x16,0x7a,0xbb,0x8c,0x5e,0x07,0x9e,0x09,0xe2,0xc8,0xa8,0x33,0x9c];
        if random==RETRY { return Err(TlsError::InvalidHandshake); }
        // The actual ClientHello always offers an empty legacy session ID.
        if buf.read_u8()? != 0 { return Err(TlsError::InvalidSessionIdLength); }
        let suite=CipherSuite::parse(buf).map_err(|_|TlsError::InvalidCipherSuite)?;
        if suite as u16 != C::CODE_POINT { return Err(TlsError::InvalidCipherSuite); }
        if buf.read_u8()? != 0 { return Err(TlsError::InvalidHandshake); }
        let length=buf.read_u16()? as usize;
        let mut offered=buf.slice(length)?;
        if !buf.is_empty() { return Err(TlsError::InvalidExtensionsLength); }
        let mut extensions=Vec::new(); let(mut version,mut key)=(false,false);
        while !offered.is_empty() {
            let kind=offered.read_u16()?; let size=offered.read_u16()? as usize;
            let mut data=offered.slice(size)?;
            let extension=match kind {
                43 if !version && size==2 => {
                    let selected=SupportedVersionsServerHello::parse(&mut data)?;
                    if selected.selected_version!=TLS13 { return Err(TlsError::InvalidSupportedVersions); }
                    version=true;ServerHelloExtension::SupportedVersions(selected)
                },
                51 if !key && size==69 => {
                    let share=KeyShareServerHello::parse(&mut data)?;
                    if share.0.group!=NamedGroup::Secp256r1 || share.0.opaque.len()!=65
                        || share.0.opaque[0]!=4 || PublicKey::from_sec1_bytes(share.0.opaque).is_err() {
                        return Err(TlsError::InvalidKeyShare);
                    }
                    key=true;ServerHelloExtension::KeyShare(share)
                },
                _ => return Err(TlsError::InvalidHandshake),
            };
            if !data.is_empty() { return Err(TlsError::InvalidExtensionsLength); }
            extensions.push(extension).map_err(|_|TlsError::InvalidHandshake)?;
        }
        if !version || !key { return Err(TlsError::InvalidHandshake); }
        Ok(Self { extensions })
    }

    pub fn key_share(&self) -> Option<&KeyShareEntry<'_>> {
        self.extensions.iter().find_map(|e| {
            if let ServerHelloExtension::KeyShare(entry) = e {
                Some(&entry.0)
            } else {
                None
            }
        })
    }

    pub fn calculate_shared_secret(&self, secret: &EphemeralSecret) -> Option<SharedSecret> {
        let server_key_share = self.key_share()?;
        let server_public_key = PublicKey::from_sec1_bytes(server_key_share.opaque).ok()?;
        Some(secret.diffie_hellman(&server_public_key))
    }

    #[allow(dead_code)]
    pub fn initialize_crypto_engine(&self, secret: &EphemeralSecret) -> Option<CryptoEngine> {
        let server_key_share = self.key_share()?;

        let group = server_key_share.group;

        let server_public_key = PublicKey::from_sec1_bytes(server_key_share.opaque).ok()?;
        let shared = secret.diffie_hellman(&server_public_key);

        Some(CryptoEngine::new(group, shared))
    }
}

#[cfg(test)]
mod sexos_negotiation_tests {
    use super::*;
    use crate::{Aes128GcmSha256,Aes256GcmSha384};
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    fn body()->std::vec::Vec<u8>{
        let public=p256::SecretKey::from_slice(&[1;32]).unwrap().public_key().to_encoded_point(false);
        let mut b=std::vec![3,3];b.extend_from_slice(&[7;32]);b.extend_from_slice(&[0,0x13,1,0]);
        let mut extensions=std::vec![0,43,0,2,3,4,0,51,0,69,0,23,0,65];extensions.extend_from_slice(public.as_bytes());
        b.extend_from_slice(&(extensions.len() as u16).to_be_bytes());b.extend_from_slice(&extensions);b
    }
    fn rejected(b:&[u8]){assert!(ServerHello::parse::<Aes128GcmSha256>(&mut ParseBuffer::new(b)).is_err());}
    #[test]fn selected_suite_must_match_the_actual_single_offered_suite(){
        let b=body();assert!(ServerHello::parse::<Aes128GcmSha256>(&mut ParseBuffer::new(&b)).is_ok());
        assert!(ServerHello::parse::<Aes256GcmSha384>(&mut ParseBuffer::new(&b)).is_err());
        for suite in [0x1302u16,0x1303,0x1304,0x00a8,0xffff]{let mut b=body();b[35..37].copy_from_slice(&suite.to_be_bytes());rejected(&b);}
    }
    #[test]fn version_compression_and_legacy_session_must_match_offer(){
        for (offset,value) in [(0,2),(1,4),(34,1),(37,1),(44,2),(45,3)]{let mut b=body();b[offset]=value;rejected(&b);}
    }
    #[test]fn key_group_point_extent_and_encoding_are_checked(){
        for (offset,value) in [(51,68),(53,24),(55,64),(56,2)]{let mut b=body();b[offset]=value;rejected(&b);}
        let mut b=body();b[56..].fill(0);b[56]=4;rejected(&b);
    }
    #[test]fn duplicate_missing_unknown_or_unoffered_extensions_reject(){
        // Duplicate versions, duplicate key share, absent versions/key share,
        // and an unknown extension cannot be silently ignored.
        let original=body();
        for range in [40..46,46..original.len()]{let mut b=original.clone();let extra=original[range].to_vec();b.extend_from_slice(&extra);let n=(b.len()-40)as u16;b[38..40].copy_from_slice(&n.to_be_bytes());rejected(&b);}
        for range in [40..46,46..original.len()]{let mut b=original.clone();b.drain(range);let n=(b.len()-40)as u16;b[38..40].copy_from_slice(&n.to_be_bytes());rejected(&b);}
        for kind in [0u16,41,44,0xffff]{let mut b=original.clone();b[40..42].copy_from_slice(&kind.to_be_bytes());rejected(&b);}
    }
    #[test]fn extension_payload_and_outer_extent_are_exact(){
        let mut b=body();b.push(0);rejected(&b);
        for(offset,value)in[(39,78),(39,80),(43,3)]{let mut b=body();b[offset]=value;rejected(&b);}
        let b=body();for len in 0..b.len(){rejected(&b[..len]);}
    }
    #[test]fn unsupported_retry_is_rejected_before_key_agreement(){
        let mut b=body();b[2..34].copy_from_slice(&[0xcf,0x21,0xad,0x74,0xe5,0x9a,0x61,0x11,0xbe,0x1d,0x8c,0x02,0x1e,0x65,0xb8,0x91,0xc2,0xa2,0x11,0x16,0x7a,0xbb,0x8c,0x5e,0x07,0x9e,0x09,0xe2,0xc8,0xa8,0x33,0x9c]);rejected(&b);
    }
}
