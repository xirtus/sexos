// SPDX-License-Identifier: GPL-3.0-only
//! Bounded iterative GIF LZW decompressor.

extern crate alloc;

use crate::Error;

const MAX_CODES: usize = 4096;

fn init_table(prefix: &mut [u16; MAX_CODES], suffix: &mut [u8; MAX_CODES], clear_code: usize) {
    for i in 0..clear_code {
        suffix[i] = i as u8;
        prefix[i] = 0;
    }
}

/// Decodes GIF LZW encoded sub-blocks into `out` indices.
pub fn decode_lzw(
    bytes: &[u8],
    mut offset: usize,
    min_code_size: u8,
    out: &mut [u8],
) -> Result<usize, Error> {
    if !(2..=8).contains(&min_code_size) {
        return Err(Error::Gif);
    }

    let clear_code = 1usize << min_code_size;
    let end_code = clear_code + 1;

    let mut prefix = [0u16; MAX_CODES];
    let mut suffix = [0u8; MAX_CODES];
    let mut stack = [0u8; MAX_CODES];

    init_table(&mut prefix, &mut suffix, clear_code);

    let mut code_size = min_code_size + 1;
    let mut next_code = end_code + 1;
    let mut max_code = 1usize << code_size;

    let mut bit_buf = 0u64;
    let mut bit_count = 0usize;
    let mut block_len = 0usize;

    let mut old_code: Option<usize> = None;
    let mut out_idx = 0usize;

    loop {
        // Read bits for one code
        while bit_count < code_size as usize {
            if block_len == 0 {
                if offset >= bytes.len() {
                    return Ok(out_idx);
                }
                block_len = bytes[offset] as usize;
                offset += 1;
                if block_len == 0 {
                    return Ok(out_idx);
                }
            }
            if offset >= bytes.len() {
                return Ok(out_idx);
            }
            bit_buf |= (bytes[offset] as u64) << bit_count;
            offset += 1;
            bit_count += 8;
            block_len -= 1;
        }

        let code = (bit_buf & ((1 << code_size) - 1)) as usize;
        bit_buf >>= code_size;
        bit_count -= code_size as usize;

        if code == clear_code {
            code_size = min_code_size + 1;
            next_code = end_code + 1;
            max_code = 1usize << code_size;
            init_table(&mut prefix, &mut suffix, clear_code);
            old_code = None;
            continue;
        }

        if code == end_code {
            break;
        }

        let mut stack_top = 0;
        let mut cur = code;

        if let Some(prev) = old_code {
            if cur >= next_code {
                if cur > next_code {
                    return Err(Error::Gif);
                }
                // Special case: code == next_code
                let mut c = prev;
                while c >= clear_code {
                    c = prefix[c] as usize;
                }
                stack[stack_top] = suffix[c];
                stack_top += 1;
                cur = prev;
            }

            while cur >= clear_code {
                if cur >= MAX_CODES || stack_top >= MAX_CODES {
                    return Err(Error::Gif);
                }
                stack[stack_top] = suffix[cur];
                stack_top += 1;
                cur = prefix[cur] as usize;
            }
            stack[stack_top] = suffix[cur];
            stack_top += 1;

            let first_char = suffix[cur];

            if next_code < MAX_CODES {
                prefix[next_code] = prev as u16;
                suffix[next_code] = first_char;
                next_code += 1;

                if next_code >= max_code && code_size < 12 {
                    code_size += 1;
                    max_code = 1usize << code_size;
                }
            }
        } else {
            if cur >= clear_code {
                return Err(Error::Gif);
            }
            stack[stack_top] = suffix[cur];
            stack_top += 1;
        }

        old_code = Some(code);

        while stack_top > 0 {
            stack_top -= 1;
            if out_idx < out.len() {
                out[out_idx] = stack[stack_top];
                out_idx += 1;
            }
        }
    }

    Ok(out_idx)
}
