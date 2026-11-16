// SPDX-License-Identifier: GPL-3.0-only
//! Allocation-free admission checks before keepass reads an untrusted header.
//! These are resource limits, not authentication; keepass still verifies the file.
const MIB: u64 = 1024 * 1024;
const LIMIT: usize = 64 * 1024;
const BAD: &str = "invalid or oversized vault header";
const COST: &str = "vault KDF exceeds Chastity's resource limits";

fn take<'a>(input: &mut &'a [u8], n: usize) -> Result<&'a [u8], &'static str> {
    let out = input.get(..n).ok_or(BAD)?;
    *input = &input[n..];
    Ok(out)
}
fn u32le(input: &mut &[u8]) -> Result<u32, &'static str> {
    Ok(u32::from_le_bytes(
        take(input, 4)?.try_into().map_err(|_| BAD)?,
    ))
}
fn number(value: &[u8]) -> Result<u64, &'static str> {
    Ok(u64::from_le_bytes(value.try_into().map_err(|_| BAD)?))
}
fn aes(rounds: u64) -> Result<(), &'static str> {
    if rounds == 0 || rounds > 10_000_000 {
        Err(COST)
    } else {
        Ok(())
    }
}

fn dictionary(mut data: &[u8], kdf: bool) -> Result<(), &'static str> {
    if take(&mut data, 2)? != [0, 1] {
        return Err(BAD);
    }
    // Borrow keys, avoiding allocations and ambiguous duplicate-key interpretation.
    let mut keys: [Option<&[u8]>; 128] = [None; 128];
    let (mut memory, mut iterations, mut lanes, mut rounds) = (None, None, None, None);
    let mut count = 0;
    loop {
        let kind = take(&mut data, 1)?[0];
        if kind == 0 {
            if !data.is_empty() {
                return Err(BAD);
            }
            break;
        }
        if count == keys.len() {
            return Err(BAD);
        }
        let n = u32le(&mut data)? as usize;
        let key = take(&mut data, n)?;
        if key.is_empty() || std::str::from_utf8(key).is_err() || keys[..count].contains(&Some(key))
        {
            return Err(BAD);
        }
        keys[count] = Some(key);
        count += 1;
        let n = u32le(&mut data)? as usize;
        let value = take(&mut data, n)?;
        match kind {
            0x04 | 0x0c if value.len() == 4 => {}
            0x05 | 0x0d if value.len() == 8 => {}
            0x08 if value.len() == 1 => {}
            0x18 | 0x42 => {}
            _ => return Err(BAD),
        }
        if kdf {
            match key {
                b"M" | b"I" | b"R" => {
                    if kind != 0x05 {
                        return Err(BAD);
                    }
                    let n = number(value)?;
                    match key {
                        b"M" => memory = Some(n),
                        b"I" => iterations = Some(n),
                        _ => rounds = Some(n),
                    }
                }
                b"P" => {
                    if kind != 0x04 {
                        return Err(BAD);
                    }
                    lanes = Some(u32::from_le_bytes(value.try_into().map_err(|_| BAD)?) as u64);
                }
                _ => {}
            }
        }
    }
    if !kdf {
        return Ok(());
    }
    match (memory, iterations, lanes, rounds) {
        (Some(m), Some(i), Some(p), None)
            if (1..=4).contains(&p)
                && m >= 8 * 1024 * p
                && m <= 64 * MIB
                && m % 1024 == 0
                && (1..=10).contains(&i)
                && m * i <= 256 * MIB =>
        {
            Ok(())
        }
        (None, None, None, Some(r)) => aes(r),
        _ => Err(COST),
    }
}

pub(crate) fn check(bytes: &[u8]) -> Result<(), &'static str> {
    if bytes.len() > 16 * MIB as usize {
        return Err("vault exceeds 16 MiB file limit");
    }
    let mut data = bytes;
    if take(&mut data, 8)? != [3, 217, 162, 154, 103, 251, 75, 181] {
        return Err(BAD);
    }
    let version = u32le(&mut data)? >> 16;
    if version != 3 && version != 4 {
        return Err("unsupported vault version");
    }
    let mut seen = [false; 256];
    let mut cost = false;
    loop {
        let kind = take(&mut data, 1)?[0] as usize;
        if seen[kind] {
            return Err(BAD);
        }
        seen[kind] = true;
        let n = if version == 4 {
            u32le(&mut data)? as usize
        } else {
            u16::from_le_bytes(take(&mut data, 2)?.try_into().map_err(|_| BAD)?) as usize
        };
        let value = take(&mut data, n)?;
        if bytes.len() - data.len() > LIMIT {
            return Err(BAD);
        }
        match kind {
            0 => {
                return if cost && (value.is_empty() || value == [13, 10, 13, 10]) {
                    Ok(())
                } else {
                    Err(BAD)
                }
            }
            3 if value.len() != 4 => return Err(BAD),
            6 if version == 3 => {
                aes(number(value)?)?;
                cost = true;
            }
            8 | 10 if version == 3 && value.len() != 4 => return Err(BAD),
            11 if version == 4 => {
                dictionary(value, true)?;
                cost = true;
            }
            12 if version == 4 => dictionary(value, false)?,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dict(m: u64, i: u64, p: u32) -> Vec<u8> {
        let mut d = vec![0, 1];
        for (kind, key, val) in [
            (5u8, b"M", m.to_le_bytes().to_vec()),
            (5, b"I", i.to_le_bytes().to_vec()),
            (4, b"P", p.to_le_bytes().to_vec()),
        ] {
            d.push(kind);
            d.extend(1u32.to_le_bytes());
            d.extend(key);
            d.extend((val.len() as u32).to_le_bytes());
            d.extend(val);
        }
        d.push(0);
        d
    }
    #[test]
    fn bounded_argon2_work() {
        for (m, i, p) in [(32 * MIB, 4, 1), (64 * MIB, 3, 4), (65536, 1, 1)] {
            assert!(dictionary(&dict(m, i, p), true).is_ok());
        }
        for (m, i, p) in [
            (u64::MAX, 1, 1),
            (64 * MIB, 10, 1),
            (MIB, u64::MAX, 1),
            (MIB, 1, u32::MAX),
        ] {
            assert!(dictionary(&dict(m, i, p), true).is_err());
        }
    }
    #[test]
    fn headers_are_bounded_and_cost_checked_before_unlock() {
        fn header(d: &[u8]) -> Vec<u8> {
            let mut b = vec![3, 217, 162, 154, 103, 251, 75, 181, 0, 0, 4, 0, 11];
            b.extend((d.len() as u32).to_le_bytes());
            b.extend(d);
            b.extend([0, 0, 0, 0, 0]);
            b
        }
        let good = header(&dict(32 * MIB, 4, 1));
        assert!(check(&good).is_ok());
        for n in 0..good.len() {
            assert!(check(&good[..n]).is_err());
        }
        assert_eq!(check(&header(&dict(u64::MAX, 1, 1))), Err(COST));
        let mut oversized = good.clone();
        oversized.resize(16 * MIB as usize + 1, 0);
        assert!(check(&oversized).is_err());
        let mut duplicate = good[..good.len() - 5].to_vec();
        duplicate.extend(&good[12..]);
        assert!(check(&duplicate).is_err());
    }
    #[test]
    fn malformed_dictionary_never_panics() {
        let d = dict(32 * MIB, 4, 1);
        for n in 0..d.len() - 1 {
            assert!(dictionary(&d[..n], true).is_err());
        }
        let mut duplicate = d[..d.len() - 1].to_vec();
        duplicate.extend(&d[2..]);
        assert!(dictionary(&duplicate, true).is_err());
        let mut short = d;
        short[8..12].copy_from_slice(&1u32.to_le_bytes());
        assert!(dictionary(&short, true).is_err());
    }
}
