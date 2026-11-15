// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;

#[derive(Debug, PartialEq)]
enum BencodeValue<'a> {
    Int(i64),
    Bytes(&'a [u8]),
    List(Vec<BencodeValue<'a>>),
    Dict(Vec<(&'a [u8], BencodeValue<'a>)>),
}

struct BencodeParser<'a> {
    data: &'a [u8],
    pos: usize,
    depth: usize,
}

impl<'a> BencodeParser<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, depth: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    fn parse_value(&mut self) -> Result<BencodeValue<'a>, ()> {
        if self.depth > 32 {
            return Err(());
        }
        match self.peek().ok_or(())? {
            b'i' => self.parse_int(),
            b'l' => self.parse_list(),
            b'd' => self.parse_dict(),
            b'0'..=b'9' => self.parse_string(),
            _ => Err(()),
        }
    }

    fn parse_int(&mut self) -> Result<BencodeValue<'a>, ()> {
        self.pos += 1; // skip 'i'
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != b'e' {
            self.pos += 1;
        }
        if self.pos >= self.data.len() {
            return Err(());
        }
        let s = core::str::from_utf8(&self.data[start..self.pos]).map_err(|_| ())?;
        self.pos += 1; // skip 'e'
        if s.is_empty() || (s.starts_with('0') && s.len() > 1) || s.starts_with("-0") {
            return Err(());
        }
        let val = s.parse::<i64>().map_err(|_| ())?;
        Ok(BencodeValue::Int(val))
    }

    fn parse_string(&mut self) -> Result<BencodeValue<'a>, ()> {
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.pos >= self.data.len() || self.data[self.pos] != b':' {
            return Err(());
        }
        let s = core::str::from_utf8(&self.data[start..self.pos]).map_err(|_| ())?;
        let len = s.parse::<usize>().map_err(|_| ())?;
        self.pos += 1; // skip ':'
        if self.data.len().saturating_sub(self.pos) < len {
            return Err(());
        }
        let slice = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(BencodeValue::Bytes(slice))
    }

    fn parse_list(&mut self) -> Result<BencodeValue<'a>, ()> {
        self.pos += 1; // skip 'l'
        self.depth += 1;
        let mut list = Vec::new();
        while self.pos < self.data.len() && self.data[self.pos] != b'e' {
            if list.len() > 256 {
                self.depth -= 1;
                return Err(());
            }
            list.push(self.parse_value()?);
        }
        self.depth -= 1;
        if self.pos >= self.data.len() {
            return Err(());
        }
        self.pos += 1; // skip 'e'
        Ok(BencodeValue::List(list))
    }

    fn parse_dict(&mut self) -> Result<BencodeValue<'a>, ()> {
        self.pos += 1; // skip 'd'
        self.depth += 1;
        let mut dict = Vec::new();
        let mut last_key: Option<&[u8]> = None;
        while self.pos < self.data.len() && self.data[self.pos] != b'e' {
            if dict.len() > 256 {
                self.depth -= 1;
                return Err(());
            }
            let key = match self.parse_string()? {
                BencodeValue::Bytes(k) => k,
                _ => {
                    self.depth -= 1;
                    return Err(());
                }
            };
            if let Some(prev) = last_key {
                if prev >= key {
                    // Non-strictly sorted keys allowed in lenient parser
                }
            }
            last_key = Some(key);
            let val = self.parse_value()?;
            dict.push((key, val));
        }
        self.depth -= 1;
        if self.pos >= self.data.len() {
            return Err(());
        }
        self.pos += 1; // skip 'e'
        Ok(BencodeValue::Dict(dict))
    }
}

fn parse_magnet(s: &str) {
    if !s.starts_with("magnet:?") {
        return;
    }
    for param in s["magnet:?".len()..].split('&') {
        if let Some((k, v)) = param.split_once('=') {
            match k {
                "xt" => {
                    if let Some(hash) = v.strip_prefix("urn:btih:") {
                        let _ = hash.len();
                    }
                }
                "dn" => {
                    let _ = v.len();
                }
                "tr" => {
                    let _ = v.len();
                }
                _ => {}
            }
        }
    }
}

fn parse_wire_packet(data: &[u8]) {
    if data.len() >= 68 && data[0] == 19 && &data[1..20] == b"BitTorrent protocol" {
        let _reserved = &data[20..28];
        let _infohash = &data[28..48];
        let _peer_id = &data[48..68];
    }
    if data.len() >= 4 {
        let msg_len = u32::from_be_bytes(data[0..4].try_into().unwrap()) as usize;
        if msg_len > 0 && data.len() >= 5 {
            let msg_id = data[4];
            let end = 5usize.saturating_add(msg_len.saturating_sub(1)).min(data.len());
            let _payload = &data[5..end];
            let _ = msg_id;
        }
    }
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. Standalone Bencode parser
    let mut parser = BencodeParser::new(data);
    let _ = parser.parse_value();

    // 2. Magnet link parsing
    if let Ok(s) = core::str::from_utf8(data) {
        parse_magnet(s);
        let prefixed = format!("magnet:?{}", s);
        parse_magnet(&prefixed);
    }

    // 3. BitTorrent wire protocol packet framing
    parse_wire_packet(data);
});
