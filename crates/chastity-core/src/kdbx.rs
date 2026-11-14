// SPDX-License-Identifier: GPL-3.0-only
//! Password-only KDBX 4.0/4.1 importer, from the official KeePass format:
//! https://keepass.info/help/kb/kdbx.html . Authentication precedes decryption.
//! Supports Argon2id/d v1.3, AES-256-CBC/ChaCha20, gzip, ChaCha20 protected XML.
//! Key files, AES-KDF, Salsa20 inner streams, attachments and custom fields are
//! refused rather than silently dropped. History is not imported.
use crate::{read_u32, take, Entry, Error, Vault, MAX_FIELD, MAX_FILE};
use aes::{
    cipher::{BlockDecrypt, KeyInit},
    Aes256,
};
use alloc::{collections::BTreeMap, string::String, vec::Vec};
use base64::{engine::general_purpose::STANDARD, Engine};
use chacha20::{
    cipher::{KeyIvInit, StreamCipher},
    ChaCha20,
};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256, Sha512};
use zeroize::Zeroizing;
const AES: &[u8] = &[
    0x31, 0xc1, 0xf2, 0xe6, 0xbf, 0x71, 0x43, 0x50, 0xbe, 0x58, 0x05, 0x21, 0x6a, 0xfc, 0x5a, 0xff,
];
const CHACHA: &[u8] = &[
    0xd6, 0x03, 0x8a, 0x2b, 0x8b, 0x6f, 0x4c, 0xb5, 0xa5, 0x24, 0x33, 0x9a, 0x31, 0xdb, 0xb5, 0x9a,
];
const ARGONID: &[u8] = &[
    0x9e, 0x29, 0x8b, 0x19, 0x56, 0xdb, 0x47, 0x73, 0xb2, 0x3d, 0xfc, 0x3e, 0xc6, 0xf0, 0xa1, 0xe6,
];
const ARGOND: &[u8] = &[
    0xef, 0x63, 0x6d, 0xdf, 0x8c, 0x29, 0x44, 0x4b, 0x91, 0xf7, 0xa9, 0xa4, 0x03, 0xe3, 0x0a, 0x0c,
];
fn hmac_key(base: &[u8], index: u64) -> Zeroizing<[u8; 64]> {
    let mut h = Sha512::new();
    h.update(index.to_le_bytes());
    h.update(base);
    Zeroizing::new(h.finalize().into())
}
fn verify(key: &[u8], pieces: &[&[u8]], tag: &[u8]) -> Result<(), Error> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).map_err(|_| Error::Format)?;
    for p in pieces {
        mac.update(p);
    }
    mac.verify_slice(tag).map_err(|_| Error::Authentication)
}
fn fields<'a>(input: &mut &'a [u8]) -> Result<(u8, &'a [u8]), Error> {
    let id = take(input, 1)?[0];
    let n = read_u32(input)? as usize;
    Ok((id, take(input, n)?))
}
fn uint(data: &[u8]) -> Result<u64, Error> {
    match data.len() {
        4 => Ok(u32::from_le_bytes(data.try_into().unwrap()) as u64),
        8 => Ok(u64::from_le_bytes(data.try_into().unwrap())),
        _ => Err(Error::Format),
    }
}
fn dictionary(mut input: &[u8]) -> Result<BTreeMap<&[u8], &[u8]>, Error> {
    if take(&mut input, 2)? != [0, 1] {
        return Err(Error::Unsupported);
    }
    let mut out = BTreeMap::new();
    loop {
        let kind = take(&mut input, 1)?[0];
        if kind == 0 {
            break;
        }
        if ![4, 5, 0x42].contains(&kind) {
            return Err(Error::Unsupported);
        }
        let n = read_u32(&mut input)? as usize;
        let name = take(&mut input, n)?;
        let n = read_u32(&mut input)? as usize;
        let value = take(&mut input, n)?;
        if out.insert(name, value).is_some() || out.len() > 16 {
            return Err(Error::Format);
        }
    }
    if !input.is_empty() {
        return Err(Error::Format);
    }
    Ok(out)
}
/// Import only current standard string fields: Title, URL, UserName, Password,
/// Notes. Unsupported data returns an explicit refusal.
pub fn import(file: &[u8], password: &[u8]) -> Result<Vault, Error> {
    if file.len() > MAX_FILE || password.len() > MAX_FIELD {
        return Err(Error::Limit);
    }
    let mut data = file;
    if take(&mut data, 8)? != [3, 0xd9, 0xa2, 0x9a, 0x67, 0xfb, 0x4b, 0xb5] {
        return Err(Error::Format);
    }
    let version = read_u32(&mut data)?;
    if ![0x40000, 0x40001].contains(&version) {
        return Err(Error::Unsupported);
    }
    let mut head = BTreeMap::new();
    loop {
        let (id, value) = fields(&mut data)?;
        if id == 0 {
            if value != [13, 10, 13, 10] {
                return Err(Error::Format);
            }
            break;
        }
        if head.insert(id, value).is_some() {
            return Err(Error::Format);
        }
    }
    let header = &file[..file.len() - data.len()];
    let checksum = take(&mut data, 32)?;
    if Sha256::digest(header).as_slice() != checksum {
        return Err(Error::Authentication);
    }
    let header_mac = take(&mut data, 32)?;
    let get = |id: u8| head.get(&id).copied().ok_or(Error::Format);
    let seed = get(4)?;
    if seed.len() != 32 {
        return Err(Error::Format);
    }
    let kdf = dictionary(get(11)?)?;
    let param = |name: &[u8]| kdf.get(name).copied().ok_or(Error::Format);
    let algo = match param(b"$UUID")? {
        v if v == ARGONID => argon2::Algorithm::Argon2id,
        v if v == ARGOND => argon2::Algorithm::Argon2d,
        _ => return Err(Error::Unsupported),
    };
    if uint(param(b"V")?)? != 0x13
        || kdf
            .keys()
            .any(|k| ![&b"$UUID"[..], b"V", b"S", b"I", b"M", b"P"].contains(k))
    {
        return Err(Error::Unsupported);
    }
    let memory = uint(param(b"M")?)?;
    let iterations = uint(param(b"I")?)?;
    let parallel = uint(param(b"P")?)?;
    if memory % 1024 != 0
        || !(8192..=268435456).contains(&memory)
        || !(1..=64).contains(&iterations)
        || memory
            .checked_mul(iterations)
            .is_none_or(|work| work > 1073741824)
        || !(1..=4).contains(&parallel)
    {
        return Err(Error::Limit);
    }
    let salt = param(b"S")?;
    if !(8..=64).contains(&salt.len()) {
        return Err(Error::Limit);
    }
    let composite = Zeroizing::new(<[u8; 32]>::from(Sha256::digest(&*Zeroizing::new(
        <[u8; 32]>::from(Sha256::digest(password)),
    ))));
    let params = argon2::Params::new(
        (memory / 1024) as u32,
        iterations as u32,
        parallel as u32,
        Some(32),
    )
    .map_err(|_| Error::Format)?;
    let transformed = crate::transform(algo, params, &*composite, salt)?;
    let mut digest = Sha256::new();
    digest.update(seed);
    digest.update(&*transformed);
    let key = Zeroizing::new(<[u8; 32]>::from(digest.finalize()));
    let mut digest = Sha512::new();
    digest.update(seed);
    digest.update(&*transformed);
    digest.update([1]);
    let base = Zeroizing::new(<[u8; 64]>::from(digest.finalize()));
    verify(&*hmac_key(&*base, u64::MAX), &[header], header_mac)?;
    let mut ciphertext = Zeroizing::new(Vec::new());
    let mut index = 0u64;
    loop {
        let tag = take(&mut data, 32)?;
        let n = read_u32(&mut data)?;
        let block = take(&mut data, n as usize)?;
        verify(
            &*hmac_key(&*base, index),
            &[&index.to_le_bytes(), &n.to_le_bytes(), block],
            tag,
        )?;
        if n == 0 {
            break;
        }
        ciphertext.extend_from_slice(block);
        if ciphertext.len() > MAX_FILE {
            return Err(Error::Limit);
        }
        index = index.checked_add(1).ok_or(Error::Limit)?;
    }
    if !data.is_empty() {
        return Err(Error::Format);
    }
    let iv = get(7)?;
    match get(2)? {
        cipher if cipher == AES => {
            if iv.len() != 16 || ciphertext.is_empty() || ciphertext.len() % 16 != 0 {
                return Err(Error::Format);
            }
            let cipher = Aes256::new_from_slice(&*key).map_err(|_| Error::Format)?;
            let mut previous = [0u8; 16];
            previous.copy_from_slice(iv);
            for block in ciphertext.chunks_mut(16) {
                let saved: [u8; 16] = block.try_into().unwrap();
                cipher.decrypt_block(block.into());
                for i in 0..16 {
                    block[i] ^= previous[i];
                }
                previous = saved;
            }
            let pad = *ciphertext.last().ok_or(Error::Format)? as usize;
            if pad == 0
                || pad > 16
                || !ciphertext[ciphertext.len() - pad..]
                    .iter()
                    .all(|b| *b as usize == pad)
            {
                return Err(Error::Authentication);
            }
            let new_len = ciphertext.len() - pad;
            ciphertext[new_len..].fill(0);
            ciphertext.truncate(new_len);
        }
        cipher if cipher == CHACHA => {
            if iv.len() != 12 {
                return Err(Error::Format);
            }
            ChaCha20::new_from_slices(&*key, iv)
                .map_err(|_| Error::Format)?
                .apply_keystream(&mut ciphertext);
        }
        _ => return Err(Error::Unsupported),
    }
    let mut plain = match uint(get(3)?)? {
        0 => ciphertext,
        1 => gunzip(&ciphertext)?,
        _ => return Err(Error::Unsupported),
    };
    parse_inner(&mut plain)
}
fn gunzip(input: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
    if input.len() < 18 || input[..3] != [0x1f, 0x8b, 8] || input[3] & 0xe0 != 0 {
        return Err(Error::Format);
    }
    let flags = input[3];
    let mut body = &input[10..input.len() - 8];
    if flags & 4 != 0 {
        let n = u16::from_le_bytes(take(&mut body, 2)?.try_into().unwrap()) as usize;
        take(&mut body, n)?;
    }
    for flag in [8, 16] {
        if flags & flag != 0 {
            let n = body.iter().position(|b| *b == 0).ok_or(Error::Format)?;
            take(&mut body, n + 1)?;
        }
    }
    if flags & 2 != 0 {
        return Err(Error::Unsupported);
    }
    // Fixed output storage prevents reallocations from abandoning plaintext,
    // and Zeroizing also covers partial output on decompression/CRC failure.
    let mut out = Zeroizing::new(alloc::vec![0u8;MAX_FILE]);
    let length = miniz_oxide::inflate::decompress_slice_iter_to_slice(
        &mut out,
        core::iter::once(body),
        false,
        true,
    )
    .map_err(|_| Error::Format)?;
    out.truncate(length);
    let mut crc = 0xffffffffu32;
    for b in out.iter() {
        crc ^= *b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 };
        }
    }
    if !crc != u32::from_le_bytes(input[input.len() - 8..input.len() - 4].try_into().unwrap())
        || out.len() as u32 != u32::from_le_bytes(input[input.len() - 4..].try_into().unwrap())
    {
        return Err(Error::Format);
    }
    Ok(out)
}
fn parse_inner(mut plain: &[u8]) -> Result<Vault, Error> {
    let mut algorithm = None;
    let mut stream_key = None;
    loop {
        let (id, value) = fields(&mut plain)?;
        match id {
            0 => {
                if !value.is_empty() {
                    return Err(Error::Format);
                }
                break;
            }
            1 => {
                if algorithm.replace(uint(value)?).is_some() {
                    return Err(Error::Format);
                }
            }
            2 => {
                if stream_key.replace(value).is_some() {
                    return Err(Error::Format);
                }
            }
            3 => return Err(Error::Unsupported),
            _ => return Err(Error::Unsupported),
        }
    }
    if algorithm != Some(3) {
        return Err(Error::Unsupported);
    }
    let key = stream_key.ok_or(Error::Format)?;
    if key.len() != 64 {
        return Err(Error::Format);
    }
    let digest = Zeroizing::new(<[u8; 64]>::from(Sha512::digest(key)));
    let mut stream =
        ChaCha20::new_from_slices(&digest[..32], &digest[32..44]).map_err(|_| Error::Format)?;
    let xml = core::str::from_utf8(plain).map_err(|_| Error::Format)?;
    let sanitized = masked_xml(xml)?;
    let safe_xml = core::str::from_utf8(&sanitized).map_err(|_| Error::Format)?;
    let doc = roxmltree::Document::parse_with_options(
        safe_xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 100000,
            ..Default::default()
        },
    )
    .map_err(|_| Error::Format)?;
    if doc.root_element().tag_name().name() != "KeePassFile" {
        return Err(Error::Format);
    }
    let mut protected = BTreeMap::new();
    for node in doc
        .descendants()
        .filter(|n| n.attribute_node("Protected").is_some())
    {
        let attr = node.attribute_node("Protected").ok_or(Error::Format)?;
        let flag = &xml[attr.range_value()];
        if flag == "False" {
            continue;
        }
        if flag != "True" {
            return Err(Error::Unsupported);
        }
        let text = raw_text(node, xml)?;
        if text.len() > MAX_FIELD * 2 {
            return Err(Error::Limit);
        }
        let mut value = Zeroizing::new(STANDARD.decode(text).map_err(|_| Error::Format)?);
        if value.len() > MAX_FIELD {
            return Err(Error::Limit);
        }
        stream.apply_keystream(&mut value);
        protected.insert(
            node.id().get(),
            Zeroizing::new(
                core::str::from_utf8(&value)
                    .map_err(|_| Error::Format)?
                    .into(),
            ),
        );
    }
    let mut vault = Vault::default();
    for node in doc
        .descendants()
        .filter(|n| n.has_tag_name("Entry") && !n.ancestors().any(|a| a.has_tag_name("History")))
    {
        if node.children().any(|n| n.has_tag_name("Binary")) {
            return Err(Error::Unsupported);
        }
        let mut fields: BTreeMap<String, Zeroizing<String>> = BTreeMap::new();
        for field in node.children().filter(|n| n.has_tag_name("String")) {
            let key_node = field
                .children()
                .find(|n| n.has_tag_name("Key"))
                .ok_or(Error::Format)?;
            let key = xml_text(raw_text(key_node, xml)?)?;
            let key = key.as_str();
            if !["Title", "URL", "UserName", "Password", "Notes"].contains(&key) {
                return Err(Error::Unsupported);
            }
            let value = field
                .children()
                .find(|n| n.has_tag_name("Value"))
                .ok_or(Error::Format)?;
            let value: Zeroizing<String> = match protected.remove(&value.id().get()) {
                Some(s) => s,
                None => xml_text(raw_text(value, xml)?)?,
            };
            if value.len() > MAX_FIELD {
                return Err(Error::Unsupported);
            }
            if fields.insert(key.into(), value).is_some() {
                return Err(Error::Format);
            }
        }
        let get = |key: &str| fields.get(key).map(|s| s.as_str()).unwrap_or("");
        let mut entry = Entry::new(get("Title"), get("URL"), get("UserName"), get("Password"))?;
        entry.notes = Zeroizing::new(get("Notes").into());
        vault.add(entry)?;
    }
    Ok(vault)
}

/// Parse only harmless text with the DOM parser. roxmltree's entity/CRLF
/// normalization otherwise allocates credential strings outside our wipe scope.
/// Positions are unchanged, so decoded field data comes only from the original
/// zeroizing plaintext through xml_text below. DTD/CDATA/comments are refused.
fn masked_xml(xml: &str) -> Result<Zeroizing<Vec<u8>>, Error> {
    if xml.contains("<!") {
        return Err(Error::Unsupported);
    }
    let mut out = Zeroizing::new(xml.as_bytes().to_vec());
    let leading = xml.len() - xml.trim_start().len();
    if xml[leading..].starts_with("<?xml ") {
        let end = xml[leading..].find("?>").ok_or(Error::Format)? + leading + 2;
        out[leading..end].fill(b' ');
    }
    let mut tag = false;
    let mut quote = 0u8;
    for byte in out.iter_mut() {
        if quote != 0 {
            if *byte == quote {
                quote = 0;
            } else {
                *byte = b'x';
            }
        } else if tag {
            match *byte {
                b'\'' | b'"' => quote = *byte,
                b'>' => tag = false,
                _ => {}
            }
        } else if *byte == b'<' {
            tag = true;
        } else if byte.is_ascii_whitespace() {
            *byte = b' ';
        } else {
            *byte = b'x';
        }
    }
    if out.windows(2).any(|s| s == b"<?") {
        return Err(Error::Unsupported);
    }
    Ok(out)
}
fn raw_text<'a>(node: roxmltree::Node<'_, '_>, xml: &'a str) -> Result<&'a str, Error> {
    let mut children = node.children();
    let Some(text) = children.next() else {
        return Ok("");
    };
    if !text.is_text() || children.next().is_some() {
        return Err(Error::Unsupported);
    }
    xml.get(text.range()).ok_or(Error::Format)
}
fn xml_text(raw: &str) -> Result<Zeroizing<String>, Error> {
    if raw.len() > MAX_FIELD * 12 {
        return Err(Error::Limit);
    }
    let mut out = Zeroizing::new(String::with_capacity(MAX_FIELD));
    let mut rest = raw;
    while !rest.is_empty() {
        let (c, used) = if let Some(entity) = rest.strip_prefix('&') {
            let n = entity.find(';').filter(|n| *n <= 12).ok_or(Error::Format)?;
            let name = &entity[..n];
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => {
                    let (digits, radix) = if let Some(s) = name.strip_prefix("#x") {
                        (s, 16)
                    } else if let Some(s) = name.strip_prefix('#') {
                        (s, 10)
                    } else {
                        return Err(Error::Unsupported);
                    };
                    char::from_u32(u32::from_str_radix(digits, radix).map_err(|_| Error::Format)?)
                        .ok_or(Error::Format)?
                }
            };
            (c, n + 2)
        } else {
            let c = rest.chars().next().ok_or(Error::Format)?;
            if c == '\r' {
                ('\n', if rest.starts_with("\r\n") { 2 } else { 1 })
            } else {
                (c, c.len_utf8())
            }
        };
        if !(c == '\t' || c == '\n' || c == '\r' || c >= ' ') {
            return Err(Error::Format);
        }
        if out.len() + c.len_utf8() > MAX_FIELD {
            return Err(Error::Limit);
        }
        out.push(c);
        rest = &rest[used..];
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xml_dom_never_receives_credential_text_and_entities_roundtrip() {
        let xml="<?xml version=\"1.0\"?><KeePassFile><Root><Group><Entry><String><Key>UserName</Key><Value>alice&amp;bob</Value></String><String><Key>Password</Key><Value Protected=\"False\">secret&lt;with&gt;&#x1f642;</Value></String></Entry></Group></Root></KeePassFile>";
        let masked = masked_xml(xml).unwrap();
        assert_eq!(masked.len(), xml.len());
        let masked = core::str::from_utf8(&masked).unwrap();
        assert!(!masked.contains("alice"));
        assert!(!masked.contains("secret"));
        assert!(!masked.contains("False"));
        let mut inner = Vec::new();
        inner.push(1);
        inner.extend_from_slice(&4u32.to_le_bytes());
        inner.extend_from_slice(&3u32.to_le_bytes());
        inner.push(2);
        inner.extend_from_slice(&64u32.to_le_bytes());
        inner.extend_from_slice(&[0; 64]);
        inner.push(0);
        inner.extend_from_slice(&0u32.to_le_bytes());
        inner.extend_from_slice(xml.as_bytes());
        let vault = parse_inner(&inner).unwrap();
        assert_eq!(&*vault.entries()[0].username, "alice&bob");
        assert_eq!(&*vault.entries()[0].password, "secret<with>🙂");
        assert!(masked_xml("<x><![CDATA[secret]]></x>").is_err());
        assert!(masked_xml("<!DOCTYPE x><x/>").is_err());
    }
    #[test]
    fn entity_decoder_is_bounded_and_normalizes_only_literal_crlf() {
        assert_eq!(
            &*xml_text("one\r\ntwo&#13;three").unwrap(),
            "one\ntwo\rthree"
        );
        for text in ["&oops;", "&#0;", "&#x110000;"] {
            assert!(xml_text(text).is_err());
        }
        assert!(xml_text(&"a".repeat(MAX_FIELD + 1)).is_err());
    }
    #[test]
    fn corrupted_gzip_checksum_and_partial_deflate_fail_closed() {
        let input = [
            0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert!(gunzip(&input).is_ok());
        let mut corrupted = input;
        corrupted[12] = 1;
        assert!(gunzip(&corrupted).is_err());
        let mut corrupted = input;
        corrupted[10] = 0xff;
        assert!(gunzip(&corrupted).is_err());
    }
}
