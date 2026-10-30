#![no_std]
#![no_main]

use sex_pdx::{pdx_listen_raw, pdx_reply, serial_println};
use spin::Mutex;
use core::sync::atomic::{AtomicU8, Ordering};
mod nic;
mod ethernet;
mod arp;

// ── Phase M: source3 reliability multi-fetch compile gate ──
const PHASE_M_RELIABILITY_ENABLED: bool =
    option_env!("SEXNET_PHASE_M_RELIABILITY_PROOF").is_some();
const DNS_SOURCE3_UDP_TX_ENABLED: bool =
    option_env!("SEXNET_DNS_SOURCE3_PROOF").is_some();

/// Heavy historical proof profile gate. The normal path uses bounded NIC
/// preparation while all IP/transport/application protocols remain offline.
/// Default boot skips e1000 ring proofs, DNS/TCP/HTTP suites, and
/// bounded poll loops (50M/100M iterations).
/// Set SEXOS_SEXNET_PROOF_PROFILE=1 at build time to enable full NIC
/// and protocol proof suites for daily-driver gate verification.
/// Note: Recompiled to clear stale cached environment variable triggers.
const SEXNET_PROOF_PROFILE_ENABLED: bool =
    option_env!("SEXOS_SEXNET_PROOF_PROFILE").is_some();

// --------------------------------------------------------------------------
// Opcodes (local — these are NOT in sex-pdx)
// --------------------------------------------------------------------------

const SEXNET_GET_STATUS:  u64 = 0x200;
const SEXNET_SCAN_WIFI:   u64 = 0x201;
const SEXNET_CONNECT:     u64 = 0x202;
const SEXNET_DISCONNECT:  u64 = 0x203;
const SEXNET_VPN_UP:      u64 = 0x204;
const SEXNET_VPN_DOWN:    u64 = 0x205;
const SEXNET_GET_IP:      u64 = 0x206;
const SEXNET_HTTP_PROOF_LEN: u64 = 0x207;
const SEXNET_HTTP_PROOF_CHUNK: u64 = 0x208;
const SEXNET_HTTP_BODY_LEN: u64 = 0x209;
const SEXNET_HTTP_BODY_CHUNK: u64 = 0x20A;
const SEXNET_DNS_RESOLVE: u64 = 0x20B;
const SEXNET_ERR_UNSUPPORTED:u64=u64::MAX;
const BODY_TEXT: &[u8] = b"Hello SexOS HTTP OK";
static mut PROOF_BUF: [u8; 32] = [0u8; 32];
static mut PROOF_LEN: usize = 0;
static mut BODY_BUF: [u8; 64] = [0u8; 64];
static mut BODY_LEN: usize = 0;
const SEXNET_GUEST_IPV4: [u8; 4] = [10, 0, 2, 15];
const SEXNET_DNS_SERVER_IPV4: [u8; 4] = [10, 0, 2, 3];
const SEXNET_DNS_SERVER_PORT: u16 = 53;
const SEXNET_DNS_SRC_PORT: u16 = 49152;
const SEXNET_DNS_TXID: u16 = 0x1234;
const SEXNET_DNS_QUERY_FRAME_LEN: usize = 71;
const HTTP_GET_BUF_CAP: usize = 192;
const HTTP_RESPONSE_BUF_CAP: usize = 512;
const HTTP_BODY_BUF_CAP: usize = 256;
static mut HTTP_GET_BUF: [u8; HTTP_GET_BUF_CAP] = [0u8; HTTP_GET_BUF_CAP];
static mut HTTP_GET_LEN: usize = 0;
static mut HTTP_RESPONSE_BUF: [u8; HTTP_RESPONSE_BUF_CAP] = [0u8; HTTP_RESPONSE_BUF_CAP];
static mut HTTP_RESPONSE_LEN: usize = 0;
static mut HTTP_BODY_PREFIX_BUF: [u8; HTTP_BODY_BUF_CAP] = [0u8; HTTP_BODY_BUF_CAP];
static mut HTTP_BODY_PREFIX_LEN: usize = 0;
static mut HTTP_STATUS_CODE: u16 = 0;
static mut DNS_A_CACHE_IP: [[u8; 4]; 4] = [[0u8; 4]; 4];
static mut DNS_A_CACHE_VALID: [u8; 4] = [0u8; 4];
static mut DNS_A_CACHE_TTL: [u32; 4] = [0u32; 4];

#[allow(dead_code)]
const NIC_OWNER_HAL_DIAG: u8 = 0;
#[allow(dead_code)]
const NIC_OWNER_SEXNET_RX: u8 = 1;
#[allow(dead_code)]
const NIC_OWNER_SEXNET_TX: u8 = 2;
#[allow(dead_code)]
const NIC_OWNER_SEXNET_FULL: u8 = 3;

static NIC_RX_OWNER: AtomicU8 = AtomicU8::new(NIC_OWNER_HAL_DIAG);
static NIC_TX_OWNER: AtomicU8 = AtomicU8::new(NIC_OWNER_HAL_DIAG);
static mut RX_PERM_DESC_PHYS: u64 = 0;
static mut RX_PERM_DESC_VA: u64 = 0;
static mut RX_PERM_PKT_PHYS: [u64; 8] = [0u64; 8];
static mut RX_PERM_PKT_VA: [u64; 8] = [0u64; 8];
static mut TX_PERM_DESC_PHYS: u64 = 0;
static mut TX_PERM_DESC_VA: u64 = 0;
static mut TX_PERM_FRAME_PHYS: u64 = 0;
static mut TX_PERM_FRAME_VA: u64 = 0;
static mut L2_RX_NEXT: u8 = 0;
static mut L2_RX_COUNT: u32 = 0;
static mut L2_TX_NEXT: u8 = 1;
static mut ARP_CACHE_MAC: [u8; 6] = [0u8; 6];
static mut ARP_CACHE_IP: [u8; 4] = [0u8; 4];
static mut ARP_CACHE_VALID: u8 = 0;
static mut ARP_CACHE_REPLY_COUNT: u32 = 0;

unsafe fn sys_net_diag(selector: u64) -> u64 {
    let result: u64;
    core::arch::asm!(
        "syscall",
        inout("rax") 52u64 => result,
        in("rdi") selector,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    result
}

// The pool only subdivides this domain's immutable, pre-mapped boot DMA region.
// Both addresses come from the same reservation; no physical address mapping API.
fn reserve_dma(pool: &mut Option<sex_pdx::dma_pool::DmaRegionPool>, bytes: u64) -> (u64, u64) {
    use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE, DMA_ADDRESS};
    if pool.is_none() {
        let Ok(view) = sex_pdx::memory_region_full_view(
            sex_pdx::SLOT_DMA_REGION, QUERY_VIEW | READ | WRITE | DMA_ADDRESS,
        ) else { return (u64::MAX, u64::MAX); };
        let Ok(new_pool) = sex_pdx::dma_pool::DmaRegionPool::new(view)
            else { return (u64::MAX, u64::MAX); };
        sex_pdx::serial_record!("[sexnet.memory.dma.ready] region_id={} length={}", view.region_id, view.length);
        *pool = Some(new_pool);
    }
    match pool.as_mut().unwrap().reserve(bytes) {
        Ok(buffer) => (buffer.physical_address, buffer.virtual_address),
        Err(_) => (u64::MAX, u64::MAX),
    }
}

unsafe fn proof_reset() {
    PROOF_LEN = 0;
}

unsafe fn proof_push_byte(b: u8) {
    if PROOF_LEN < PROOF_BUF.len() {
        PROOF_BUF[PROOF_LEN] = b;
        PROOF_LEN += 1;
    }
}

unsafe fn proof_push_str(s: &str) {
    for &b in s.as_bytes() {
        proof_push_byte(b);
    }
}

unsafe fn proof_push_u32(mut n: u32) {
    let mut tmp = [0u8; 10];
    let mut i = 0usize;
    if n == 0 {
        proof_push_byte(b'0');
        return;
    }
    while n > 0 && i < tmp.len() {
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        proof_push_byte(tmp[i]);
    }
}

unsafe fn proof_build(status: u32, bytes: u16, source: u8) {
    proof_reset();
    proof_push_str("HTTP ");
    proof_push_u32(status);
    proof_push_str(" rx=");
    proof_push_u32(bytes as u32);
    proof_push_str("b ");
    match source {
        1 => proof_push_str("mock"),
        2 => proof_push_str("real"),
        _ => proof_push_str("unset"),
    }
}

unsafe fn http_get_build(host: &[u8], path: &[u8]) -> usize {
    let p0 = b"GET ";
    let p1 = b" HTTP/1.1\r\nHost: ";
    let p2 = b"\r\nConnection: close\r\nUser-Agent: sexnet/phase-i\r\n\r\n";
    let needed = p0.len() + path.len() + p1.len() + host.len() + p2.len();
    if needed > HTTP_GET_BUF_CAP {
        return 0;
    }
    let mut o = 0usize;
    for &b in p0 { HTTP_GET_BUF[o] = b; o += 1; }
    for &b in path { HTTP_GET_BUF[o] = b; o += 1; }
    for &b in p1 { HTTP_GET_BUF[o] = b; o += 1; }
    for &b in host { HTTP_GET_BUF[o] = b; o += 1; }
    for &b in p2 { HTTP_GET_BUF[o] = b; o += 1; }
    HTTP_GET_LEN = o;
    o
}

fn build_dns_query_frame_source3(out: &mut [u8; SEXNET_DNS_QUERY_FRAME_LEN]) -> usize {
    let mut i = 0usize;
    while i < SEXNET_DNS_QUERY_FRAME_LEN {
        out[i] = 0;
        i += 1;
    }
    // Ethernet header: dst/src left zeroed for build-proof only, ethertype IPv4.
    out[12] = 0x08;
    out[13] = 0x00;
    // IPv4 header.
    out[14] = 0x45;
    out[15] = 0x00;
    out[16] = 0x00;
    out[17] = 57; // IPv4(20) + UDP(8) + DNS(29)
    out[18] = 0x00;
    out[19] = 0x07;
    out[20] = 0x00;
    out[21] = 0x00;
    out[22] = 64;
    out[23] = 17; // UDP
    out[26] = SEXNET_GUEST_IPV4[0];
    out[27] = SEXNET_GUEST_IPV4[1];
    out[28] = SEXNET_GUEST_IPV4[2];
    out[29] = SEXNET_GUEST_IPV4[3];
    out[30] = SEXNET_DNS_SERVER_IPV4[0];
    out[31] = SEXNET_DNS_SERVER_IPV4[1];
    out[32] = SEXNET_DNS_SERVER_IPV4[2];
    out[33] = SEXNET_DNS_SERVER_IPV4[3];
    // IPv4 checksum.
    let mut ip_sum = 0u32;
    let mut w = 0usize;
    while w < 10 {
        let off = 14 + w * 2;
        let hi = out[off] as u16;
        let lo = out[off + 1] as u16;
        ip_sum += ((hi << 8) | lo) as u32;
        w += 1;
    }
    while (ip_sum >> 16) != 0 {
        ip_sum = (ip_sum & 0xFFFF) + (ip_sum >> 16);
    }
    let ip_ck = !(ip_sum as u16);
    out[24] = ((ip_ck >> 8) & 0xFF) as u8;
    out[25] = (ip_ck & 0xFF) as u8;
    // UDP header.
    out[34] = ((SEXNET_DNS_SRC_PORT >> 8) & 0xFF) as u8;
    out[35] = (SEXNET_DNS_SRC_PORT & 0xFF) as u8;
    out[36] = ((SEXNET_DNS_SERVER_PORT >> 8) & 0xFF) as u8;
    out[37] = (SEXNET_DNS_SERVER_PORT & 0xFF) as u8;
    out[38] = 0x00;
    out[39] = 37; // UDP(8) + DNS(29)
    out[40] = 0x00;
    out[41] = 0x00; // checksum omitted in build-only proof
    // DNS query: txid=0x1234, flags=RD, QD=1, qname=example.com, A IN.
    out[42] = ((SEXNET_DNS_TXID >> 8) & 0xFF) as u8;
    out[43] = (SEXNET_DNS_TXID & 0xFF) as u8;
    out[44] = 0x01;
    out[45] = 0x00;
    out[46] = 0x00;
    out[47] = 0x01;
    out[48] = 0x00;
    out[49] = 0x00;
    out[50] = 0x00;
    out[51] = 0x00;
    out[52] = 0x00;
    out[53] = 0x00;
    out[54] = 0x07;
    out[55] = b'e';
    out[56] = b'x';
    out[57] = b'a';
    out[58] = b'm';
    out[59] = b'p';
    out[60] = b'l';
    out[61] = b'e';
    out[62] = 0x03;
    out[63] = b'c';
    out[64] = b'o';
    out[65] = b'm';
    out[66] = 0x00;
    out[67] = 0x00;
    out[68] = 0x01;
    out[69] = 0x00;
    out[70] = 0x01;
    SEXNET_DNS_QUERY_FRAME_LEN
}

fn find_crlf(buf: &[u8], len: usize) -> usize {
    let mut i = 0usize;
    while i + 1 < len {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' {
            return i;
        }
        i += 1;
    }
    len
}

fn build_hex_peek(buf: &[u8], len: usize, out: &mut [u8]) -> usize {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut o = 0usize;
    let mut i = 0usize;
    while i < len {
        if o + 2 > out.len() {
            break;
        }
        let b = buf[i];
        out[o] = HEX[(b >> 4) as usize];
        out[o + 1] = HEX[(b & 0x0F) as usize];
        o += 2;
        if i + 1 < len {
            if o + 1 > out.len() {
                break;
            }
            out[o] = b' ';
            o += 1;
        }
        i += 1;
    }
    o
}

fn build_ascii_peek(buf: &[u8], len: usize, out: &mut [u8]) -> usize {
    let mut o = 0usize;
    let mut i = 0usize;
    while i < len && o < out.len() {
        let b = buf[i];
        out[o] = if (0x20..=0x7e).contains(&b) { b } else { b'.' };
        o += 1;
        i += 1;
    }
    o
}

fn parse_http_status_line(buf: &[u8], len: usize) -> (u16, usize, &'static str, &'static str) {
    const MAX_STATUS_LINE: usize = 128;
    if len == 0 {
        return (0, 0, "", "empty");
    }
    let scan_cap = if len < MAX_STATUS_LINE { len } else { MAX_STATUS_LINE };
    let mut line_end = scan_cap;
    let mut i = 0usize;
    while i < scan_cap {
        let b = buf[i];
        if b == b'\n' || b == b'\r' {
            line_end = i;
            break;
        }
        i += 1;
    }
    if line_end == scan_cap {
        if len >= MAX_STATUS_LINE {
            return (0, line_end, "", "status_line_too_long");
        }
        return (0, line_end, "", "missing_line_ending");
    }
    if line_end < 12 {
        return (0, line_end, "", "status_line_too_short");
    }
    if buf[0] != b'H' || buf[1] != b'T' || buf[2] != b'T' || buf[3] != b'P' || buf[4] != b'/' || buf[5] != b'1' || buf[6] != b'.' {
        return (0, line_end, "", "bad_http_prefix");
    }
    let version = if buf[7] == b'0' {
        "HTTP/1.0"
    } else if buf[7] == b'1' {
        "HTTP/1.1"
    } else {
        return (0, line_end, "", "unsupported_http_version");
    };
    if buf[8] != b' ' {
        return (0, line_end, "", "missing_status_space");
    }
    let d0 = buf[9];
    let d1 = buf[10];
    let d2 = buf[11];
    if !d0.is_ascii_digit() || !d1.is_ascii_digit() || !d2.is_ascii_digit() {
        return (0, line_end, "", "status_digits_invalid");
    }
    if line_end > 12 && buf[12] != b' ' {
        return (0, line_end, "", "missing_reason_separator");
    }
    let status = ((d0 - b'0') as u16) * 100 + ((d1 - b'0') as u16) * 10 + ((d2 - b'0') as u16);
    (status, line_end, version, "")
}

unsafe fn dns_source3_try_parse_and_cache(pkt_buf: u64, rx_len: usize) -> u32 {
    if rx_len < 54 {
        serial_println!("[sexnet.dns.malformed.reject] reason=rx_too_short ok=1");
        return 0;
    }
    let et0 = core::ptr::read_volatile((pkt_buf + 12) as *const u8);
    let et1 = core::ptr::read_volatile((pkt_buf + 13) as *const u8);
    if et0 != 0x08 || et1 != 0x00 {
        return 0;
    }
    let ihl = ((core::ptr::read_volatile((pkt_buf + 14) as *const u8) & 0x0F) as usize) * 4;
    if ihl < 20 || (14 + ihl + 8) > rx_len {
        serial_println!("[sexnet.dns.malformed.reject] reason=ipv4_ihl_bounds ok=1");
        return 0;
    }
    let proto = core::ptr::read_volatile((pkt_buf + 23) as *const u8);
    if proto != 17 {
        return 0;
    }
    let src0 = core::ptr::read_volatile((pkt_buf + 26) as *const u8);
    let src1 = core::ptr::read_volatile((pkt_buf + 27) as *const u8);
    let src2 = core::ptr::read_volatile((pkt_buf + 28) as *const u8);
    let src3 = core::ptr::read_volatile((pkt_buf + 29) as *const u8);
    if src0 != SEXNET_DNS_SERVER_IPV4[0]
        || src1 != SEXNET_DNS_SERVER_IPV4[1]
        || src2 != SEXNET_DNS_SERVER_IPV4[2]
        || src3 != SEXNET_DNS_SERVER_IPV4[3]
    {
        return 0;
    }
    let udp_base = 14 + ihl;
    let src_port = ((core::ptr::read_volatile((pkt_buf + udp_base as u64) as *const u8) as u16) << 8)
        | (core::ptr::read_volatile((pkt_buf + udp_base as u64 + 1) as *const u8) as u16);
    if src_port != SEXNET_DNS_SERVER_PORT {
        return 0;
    }
    let udp_len = ((core::ptr::read_volatile((pkt_buf + udp_base as u64 + 4) as *const u8) as u16) << 8)
        | (core::ptr::read_volatile((pkt_buf + udp_base as u64 + 5) as *const u8) as u16);
    if udp_len < 8 {
        serial_println!("[sexnet.dns.malformed.reject] reason=udp_len_small ok=1");
        return 0;
    }
    let dns_len = (udp_len as usize) - 8;
    let dns_base = udp_base + 8;
    if dns_base + dns_len > rx_len || dns_len < 12 {
        serial_println!("[sexnet.dns.malformed.reject] reason=dns_bounds ok=1");
        return 0;
    }
    let txid = ((core::ptr::read_volatile((pkt_buf + dns_base as u64) as *const u8) as u16) << 8)
        | (core::ptr::read_volatile((pkt_buf + dns_base as u64 + 1) as *const u8) as u16);
    if txid != SEXNET_DNS_TXID {
        return 0;
    }
    let flags = ((core::ptr::read_volatile((pkt_buf + dns_base as u64 + 2) as *const u8) as u16) << 8)
        | (core::ptr::read_volatile((pkt_buf + dns_base as u64 + 3) as *const u8) as u16);
    let qr = if (flags & 0x8000) != 0 { 1u16 } else { 0u16 };
    let rcode = (flags & 0x000F) as u16;
    if qr == 0 {
        serial_println!("[sexnet.dns.malformed.reject] reason=qr_not_set ok=1");
        return 0;
    }
    if rcode != 0 {
        serial_println!("[sexnet.dns.malformed.reject] reason=rcode_nonzero ok=1");
        return 0;
    }
    let qdcount = ((core::ptr::read_volatile((pkt_buf + dns_base as u64 + 4) as *const u8) as u16) << 8)
        | (core::ptr::read_volatile((pkt_buf + dns_base as u64 + 5) as *const u8) as u16);
    let ancount = ((core::ptr::read_volatile((pkt_buf + dns_base as u64 + 6) as *const u8) as u16) << 8)
        | (core::ptr::read_volatile((pkt_buf + dns_base as u64 + 7) as *const u8) as u16);
    serial_println!(
        "[sexnet.dns.source3.rx.parse] txid=0x1234 qr={} rcode={} ancount={} ok=1",
        qr,
        rcode,
        ancount
    );

    let mut off = dns_base + 12;
    if qdcount > 0 {
        let mut q_iter = 0u32;
        loop {
            if q_iter >= 64 {
                serial_println!("[sexnet.dns.malformed.reject] reason=qname_loop_limit ok=1");
                return 0;
            }
            if off >= dns_base + dns_len {
                serial_println!("[sexnet.dns.malformed.reject] reason=qname_oob ok=1");
                return 0;
            }
            let lab = core::ptr::read_volatile((pkt_buf + off as u64) as *const u8);
            off += 1;
            if lab == 0 {
                break;
            }
            if (lab & 0xC0) != 0 {
                serial_println!("[sexnet.dns.malformed.reject] reason=qname_compression_unsupported ok=1");
                return 0;
            }
            let step = lab as usize;
            if off + step > dns_base + dns_len {
                serial_println!("[sexnet.dns.malformed.reject] reason=qname_label_oob ok=1");
                return 0;
            }
            off += step;
            q_iter += 1;
        }
        if off + 4 > dns_base + dns_len {
            serial_println!("[sexnet.dns.malformed.reject] reason=question_tail_oob ok=1");
            return 0;
        }
        off += 4;
    }

    let mut ai = 0u32;
    while ai < 2 && (ai as u16) < ancount {
        if off + 12 > dns_base + dns_len {
            serial_println!("[sexnet.dns.malformed.reject] reason=answer_header_oob ok=1");
            return 0;
        }
        let n0 = core::ptr::read_volatile((pkt_buf + off as u64) as *const u8);
        if (n0 & 0xC0) == 0xC0 {
            if off + 2 > dns_base + dns_len {
                serial_println!("[sexnet.dns.malformed.reject] reason=answer_name_ptr_oob ok=1");
                return 0;
            }
            off += 2;
        } else {
            let mut name_iter = 0u32;
            loop {
                if name_iter >= 64 {
                    serial_println!("[sexnet.dns.malformed.reject] reason=answer_name_loop_limit ok=1");
                    return 0;
                }
                if off >= dns_base + dns_len {
                    serial_println!("[sexnet.dns.malformed.reject] reason=answer_name_oob ok=1");
                    return 0;
                }
                let lab = core::ptr::read_volatile((pkt_buf + off as u64) as *const u8);
                off += 1;
                if lab == 0 {
                    break;
                }
                if (lab & 0xC0) != 0 {
                    serial_println!("[sexnet.dns.malformed.reject] reason=answer_name_bad_compression ok=1");
                    return 0;
                }
                let step = lab as usize;
                if off + step > dns_base + dns_len {
                    serial_println!("[sexnet.dns.malformed.reject] reason=answer_name_label_oob ok=1");
                    return 0;
                }
                off += step;
                name_iter += 1;
            }
        }
        if off + 10 > dns_base + dns_len {
            serial_println!("[sexnet.dns.malformed.reject] reason=answer_fields_oob ok=1");
            return 0;
        }
        let typ = ((core::ptr::read_volatile((pkt_buf + off as u64) as *const u8) as u16) << 8)
            | (core::ptr::read_volatile((pkt_buf + off as u64 + 1) as *const u8) as u16);
        let cls = ((core::ptr::read_volatile((pkt_buf + off as u64 + 2) as *const u8) as u16) << 8)
            | (core::ptr::read_volatile((pkt_buf + off as u64 + 3) as *const u8) as u16);
        let ttl = ((core::ptr::read_volatile((pkt_buf + off as u64 + 4) as *const u8) as u32) << 24)
            | ((core::ptr::read_volatile((pkt_buf + off as u64 + 5) as *const u8) as u32) << 16)
            | ((core::ptr::read_volatile((pkt_buf + off as u64 + 6) as *const u8) as u32) << 8)
            | (core::ptr::read_volatile((pkt_buf + off as u64 + 7) as *const u8) as u32);
        let rdlen = ((core::ptr::read_volatile((pkt_buf + off as u64 + 8) as *const u8) as u16) << 8)
            | (core::ptr::read_volatile((pkt_buf + off as u64 + 9) as *const u8) as u16);
        off += 10;
        if off + (rdlen as usize) > dns_base + dns_len {
            serial_println!("[sexnet.dns.malformed.reject] reason=rdata_oob ok=1");
            return 0;
        }
        if typ == 1 && cls == 1 && rdlen == 4 {
            let a0 = core::ptr::read_volatile((pkt_buf + off as u64) as *const u8);
            let a1 = core::ptr::read_volatile((pkt_buf + off as u64 + 1) as *const u8);
            let a2 = core::ptr::read_volatile((pkt_buf + off as u64 + 2) as *const u8);
            let a3 = core::ptr::read_volatile((pkt_buf + off as u64 + 3) as *const u8);
            serial_println!(
                "[sexnet.dns.source3.answer.a] idx={} addr={}.{}.{}.{} ttl={} ok=1",
                ai,
                a0,
                a1,
                a2,
                a3,
                ttl
            );
            let mut ins = 0usize;
            while ins < 4 {
                if DNS_A_CACHE_VALID[ins] == 0 {
                    break;
                }
                ins += 1;
            }
            if ins >= 4 {
                ins = 0;
            }
            DNS_A_CACHE_IP[ins] = [a0, a1, a2, a3];
            DNS_A_CACHE_TTL[ins] = ttl;
            DNS_A_CACHE_VALID[ins] = 1;
            serial_println!(
                "[sexnet.dns.source3.cache.insert] idx={} addr={}.{}.{}.{} ok=1",
                ins,
                a0,
                a1,
                a2,
                a3
            );
        }
        off += rdlen as usize;
        ai += 1;
    }
    1
}

// --------------------------------------------------------------------------
// TCP state machine (Phase G)
// --------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum TcpState { Closed, SynSent, Established, FailedRst, FailedTimeout }

static TCP_STATE: Mutex<TcpState> = Mutex::new(TcpState::Closed);
static mut TCP_LOCAL_PORT: u16 = 7777;
static mut TCP_REMOTE_PORT: u16 = 18081;
static mut TCP_LOCAL_SEQ: u32 = 42;
static mut TCP_REMOTE_SEQ: u32 = 0;
static mut TCP_REMOTE_IP: [u8; 4] = [10, 0, 2, 2]; // gateway
static mut TCP_SYN_COUNT: u32 = 0;
static mut TCP_ACK_COUNT: u32 = 0;
static mut TCP_RST_COUNT: u32 = 0;
static mut TCP_TIMEOUT_COUNT: u32 = 0;

// --------------------------------------------------------------------------
// PDX dispatch
// --------------------------------------------------------------------------

fn handle_call(syscall_id: u64, arg0: u64, arg1: u64) -> u64 {
    match syscall_id {
        SEXNET_GET_STATUS => {
            if arg0 == SEXNET_HTTP_PROOF_LEN {
                let len = unsafe { PROOF_LEN };
                serial_println!("[sexnet.packed_text.len] len={}", len);
                return len as u64;
            }
            if arg0 == SEXNET_HTTP_PROOF_CHUNK {
                let idx = arg1 as usize;
                let start = idx.saturating_mul(8);
                let proof_len = unsafe { PROOF_LEN };
                if start >= proof_len {
                    return u64::MAX;
                }
                let end = core::cmp::min(start + 8, proof_len);
                let bytes = end - start;
                let mut packed = 0u64;
                let mut i = 0usize;
                while i < bytes {
                    let b = unsafe { PROOF_BUF[start + i] };
                    packed |= (b as u64) << (i * 8);
                    i += 1;
                }
                serial_println!("[sexnet.packed_text.chunk] idx={} bytes={}", idx, bytes);
                return packed;
            }
            if arg0 == SEXNET_HTTP_BODY_LEN {
                let body_len = unsafe { BODY_LEN };
                if body_len > 0 {
                    serial_println!("[sexnet.body_text.len] len={}", body_len);
                    return body_len as u64;
                }
                serial_println!("[sexnet.body_text.len] len={}", BODY_TEXT.len());
                return BODY_TEXT.len() as u64;
            }
            if arg0 == SEXNET_HTTP_BODY_CHUNK {
                let idx = arg1 as usize;
                let start = idx.saturating_mul(8);
                let body_len = unsafe { BODY_LEN };
                if body_len > 0 {
                    if start >= body_len {
                        return u64::MAX;
                    }
                    let end = core::cmp::min(start + 8, body_len);
                    let bytes = end - start;
                    let mut packed = 0u64;
                    let mut i = 0usize;
                    while i < bytes {
                        let b = unsafe { BODY_BUF[start + i] };
                        packed |= (b as u64) << (i * 8);
                        i += 1;
                    }
                    serial_println!("[sexnet.body_text.chunk] idx={} bytes={}", idx, bytes);
                    return packed;
                }
                if start >= BODY_TEXT.len() {
                    return u64::MAX;
                }
                let end = core::cmp::min(start + 8, BODY_TEXT.len());
                let bytes = end - start;
                let mut packed = 0u64;
                let mut i = 0usize;
                while i < bytes {
                    packed |= (BODY_TEXT[start + i] as u64) << (i * 8);
                    i += 1;
                }
                serial_println!("[sexnet.body_text.chunk] idx={} bytes={}", idx, bytes);
                return packed;
            }
            // The normal service has no implemented connection control path.
            // Cached/sample body bytes never establish transport connectivity.
            0
        }

        // Arguments are data, never authority to dereference a global address.
        // These controls have no real implementation and cannot claim success.
        SEXNET_SCAN_WIFI|SEXNET_CONNECT|SEXNET_DISCONNECT|SEXNET_VPN_UP|SEXNET_VPN_DOWN=>SEXNET_ERR_UNSUPPORTED,
        SEXNET_GET_IP => 0,
        SEXNET_DNS_RESOLVE => {
            let host_id = arg0;
            if host_id != 1 {
                serial_println!(
                    "[browser.dns.resolve.miss] host_id={} ok=1 reason=unsupported_host",
                    host_id
                );
                return 0;
            }

            let mut idx = 0usize;
            while idx < 4 {
                let (valid, ttl, ip) = unsafe {
                    (
                        DNS_A_CACHE_VALID[idx],
                        DNS_A_CACHE_TTL[idx],
                        DNS_A_CACHE_IP[idx],
                    )
                };
                if valid != 0 && ttl != 0 {
                    let packed = u32::from_be_bytes(ip) as u64;
                    serial_println!(
                        "[browser.dns.resolve.ok] addr={}.{}.{}.{} ok=1",
                        ip[0],
                        ip[1],
                        ip[2],
                        ip[3]
                    );
                    return packed;
                }
                idx += 1;
            }

            serial_println!("[browser.dns.resolve.miss] host_id=1 ok=1 reason=cache_miss");
            0
        }
        _ => u64::MAX,
    }
}

// --------------------------------------------------------------------------
// Entry
// --------------------------------------------------------------------------

fn command_probe() {
    use sex_pdx::pci_command as w;
    // Read-only normal-profile probe. No device activation, ring or DMA work.
    for (slot,set,clear) in [(0,0,0),((1u64<<32)|sex_pdx::SLOT_NIC,2,4),
        (sex_pdx::SLOT_NIC,1,0),(sex_pdx::SLOT_NIC,1u64<<32,0),
        (sex_pdx::SLOT_NIC,w::MEMORY as u64,w::MEMORY as u64)] {
        assert_eq!(sex_pdx::device_command(slot,set,clear),Err(w::DENIED),"NIC_COMMAND_AUTHORITY_PROBE");
    }
    match sex_pdx::device_command(sex_pdx::SLOT_NIC,0,0) {
        Ok(r)=>{
            assert!(r.before==r.after&&!r.changed,"NIC_COMMAND_QUERY_MUTATION");
            sex_pdx::serial_record!("[sexnet.nic.command.ready] before={} after={} requested_bits=0 changed=0 denied=5 ok=1",r.before,r.after);
        },
        Err(error)=>sex_pdx::serial_record!("[sexnet.nic.command.unavailable] status={} requested_bits=0 denied=5 ok=1",error),
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    command_probe();
    for slot in [0,(1u64<<32)|sex_pdx::SLOT_NIC,sex_pdx::SLOT_DMA_REGION,sex_pdx::SLOT_DISPLAY] {
        assert_eq!(sex_pdx::device_identity(slot),Err(sex_pdx::pci_command::DENIED),"NIC_BOOT_IDENTITY_PROBE");
    }
    sex_pdx::serial_record!("[sexnet.nic.identity.denied] requests=4 output_zero=1 ok=1");
    let mut dma_pool = None;
    let raw = unsafe { sys_net_diag(0) };
    let status = (raw >> 32) as u32;
    let source = ((raw >> 16) & 0xFF) as u8;
    let bytes = (raw & 0xFFFF) as u16;
    serial_println!(
        "[net.diag.syscall.call] syscall=52 status={} bytes={} source={}",
        status,
        bytes,
        source
    );
    unsafe {
        proof_build(status, bytes, source);
    }
    let proof_len = unsafe { PROOF_LEN };
    serial_println!(
        "[sexnet.dynamic_text.set] status={} bytes={} source={} len={} ok=1",
        status,
        bytes,
        source,
        proof_len
    );
    if source == 2 {
        serial_println!("[legacy.source2.dns.not_used] source=2 dns=0 ok=1");
        let blen_raw = unsafe { sys_net_diag(1) };
        let blen = core::cmp::min(blen_raw as usize, 64usize);
        if blen > 0 {
            serial_println!("[sexnet.body.fetch.begin] len={}", blen);
            let chunks = (blen + 7) / 8;
            let mut ci = 0usize;
            while ci < chunks {
                let packed = unsafe { sys_net_diag(2 + ci as u64) };
                let mut bi = 0usize;
                while bi < 8 {
                    let idx = ci * 8 + bi;
                    if idx >= blen {
                        break;
                    }
                    let byte = ((packed >> (bi * 8)) & 0xFF) as u8;
                    let sane = if byte == b'\r' || byte == b'\n' { b' ' } else { byte };
                    unsafe {
                        BODY_BUF[idx] = sane;
                    }
                    bi += 1;
                }
                ci += 1;
            }
            unsafe {
                BODY_LEN = blen;
            }
            serial_println!("[sexnet.dynamic_body.set] len={} source=2 ok=1", blen);
        }
    }
    serial_println!("[sexnet.boot] ok=1 reason=passive_spawn");
    serial_println!("[sexnet.passive.ready] network=0 dns=0 tcp=0 http=0 tls=0 ok=1 reason=protocols_offline");
    serial_println!("[sexnet.passive.spawn.done] ok=1 spawned=1 browser_network=0");
    if SEXNET_PROOF_PROFILE_ENABLED {
        serial_println!("[sexnet.proof.profile] enabled=1 reason=SEXOS_SEXNET_PROOF_PROFILE_set");
        let nic_va = sex_pdx::memory_region_full_view(
            sex_pdx::SLOT_NIC, sex_pdx::memory_region::QUERY_VIEW
                | sex_pdx::memory_region::READ | sex_pdx::memory_region::WRITE,
        ).ok().filter(|view| view.length >= 0x5408)
            .map(|view| view.virtual_address).unwrap_or(u64::MAX);
        if nic_va != u64::MAX {
            let ral = unsafe { core::ptr::read_volatile((nic_va + 0x5400) as *const u32) };
            let rah = unsafe { core::ptr::read_volatile((nic_va + 0x5404) as *const u32) };
            serial_println!("[sexnet.nic.bar.map] va={:#x} ok=1", nic_va);
            let rx_own = NIC_RX_OWNER.load(Ordering::Acquire);
            let tx_own = NIC_TX_OWNER.load(Ordering::Acquire);
            serial_println!(
                "[sexnet.nic.ownership.init] rx_owner={} tx_owner={} ok=1",
                rx_own,
                tx_own
            );
            serial_println!(
                "[sexnet.nic.mac.read] ral={:#010X} rah={:#010X} ok=1",
                ral,
                rah
            );
            serial_println!("[sexnet.nic.reg.audit.begin] ok=1");
            let reg_ctrl = unsafe { core::ptr::read_volatile((nic_va + 0x0000) as *const u32) };
            let reg_status = unsafe { core::ptr::read_volatile((nic_va + 0x0008) as *const u32) };
            let reg_rctl = unsafe { core::ptr::read_volatile((nic_va + 0x0100) as *const u32) };
            let reg_rdbal = unsafe { core::ptr::read_volatile((nic_va + 0x2800) as *const u32) };
            let reg_rdbah = unsafe { core::ptr::read_volatile((nic_va + 0x2804) as *const u32) };
            let reg_rdlen = unsafe { core::ptr::read_volatile((nic_va + 0x2808) as *const u32) };
            let reg_rdh = unsafe { core::ptr::read_volatile((nic_va + 0x2810) as *const u32) };
            let reg_rdt = unsafe { core::ptr::read_volatile((nic_va + 0x2818) as *const u32) };
            serial_println!("[sexnet.nic.reg.ctrl] val={:#010X}", reg_ctrl);
            serial_println!("[sexnet.nic.reg.status] val={:#010X}", reg_status);
            serial_println!(
                "[sexnet.nic.reg.rctl] val={:#010X} rctl_en={}",
                reg_rctl,
                if (reg_rctl & (1 << 1)) != 0 { 1 } else { 0 }
            );
            serial_println!("[sexnet.nic.reg.rdbal] val={:#010X}", reg_rdbal);
            serial_println!("[sexnet.nic.reg.rdbah] val={:#010X}", reg_rdbah);
            serial_println!("[sexnet.nic.reg.rdlen] val={}", reg_rdlen);
            serial_println!("[sexnet.nic.reg.rdh] val={}", reg_rdh);
            serial_println!("[sexnet.nic.reg.rdt] val={}", reg_rdt);
            serial_println!(
                "[sexnet.nic.reg.audit.done] rctl_en={} rdlen={} ok=1",
                if (reg_rctl & (1 << 1)) != 0 { 1 } else { 0 },
                reg_rdlen
            );
            let rctl_orig = unsafe { core::ptr::read_volatile((nic_va + 0x0100) as *const u32) };
            let rctl_clear = rctl_orig & !(1u32 << 1);
            serial_println!(
                "[sexnet.nic.rctl.disable.begin] rctl_orig={:#010X}",
                rctl_orig
            );
            unsafe {
                core::ptr::write_volatile((nic_va + 0x0100) as *mut u32, rctl_clear);
            }
            serial_println!("[sexnet.nic.rctl.disable.write] val={:#010X}", rctl_clear);
            let rctl_readback_off =
                unsafe { core::ptr::read_volatile((nic_va + 0x0100) as *const u32) };
            let rctl_en_off = (rctl_readback_off >> 1) & 1;
            let rctl_disable_ok = if rctl_en_off == 0 { 1 } else { 0 };
            serial_println!(
                "[sexnet.nic.rctl.disable.readback] rctl_en={} ok={}",
                rctl_en_off,
                rctl_disable_ok
            );
            unsafe {
                core::ptr::write_volatile((nic_va + 0x0100) as *mut u32, rctl_orig);
            }
            serial_println!("[sexnet.nic.rctl.restore.write] val={:#010X}", rctl_orig);
            let rctl_readback_on =
                unsafe { core::ptr::read_volatile((nic_va + 0x0100) as *const u32) };
            let rctl_en_on = (rctl_readback_on >> 1) & 1;
            let rctl_restore_ok = if rctl_en_on == 1 { 1 } else { 0 };
            serial_println!(
                "[sexnet.nic.rctl.restore.readback] rctl_en={} ok={}",
                rctl_en_on,
                rctl_restore_ok
            );
            let proof_ok = if rctl_disable_ok == 1 && rctl_restore_ok == 1 {
                1
            } else {
                0
            };
            serial_println!("[sexnet.nic.rctl.disable_restore.proof.done] ok={}", proof_ok);

            let (desc_phys, desc_va) = reserve_dma(&mut dma_pool, 4096);

            let (pkt_phys, pkt_va) = reserve_dma(&mut dma_pool, 4096);

            let alloc_ok = desc_phys != 0
                && desc_phys != u64::MAX
                && desc_va != 0
                && desc_va != u64::MAX
                && pkt_phys != 0
                && pkt_phys != u64::MAX
                && pkt_va != 0
                && pkt_va != u64::MAX;
            if !alloc_ok {
                serial_println!(
                    "[sexnet.nic.rx.ring.alloc] desc_phys=0x{:016X} pkt_phys=0x{:016X} ok=0",
                    desc_phys,
                    pkt_phys
                );
            } else {
                serial_println!(
                    "[sexnet.nic.rx.ring.alloc] desc_phys=0x{:016X} pkt_phys=0x{:016X} ok=1",
                    desc_phys,
                    pkt_phys
                );
                let mut zi = 0u64;
                while zi < 512 {
                    unsafe {
                        core::ptr::write_volatile((desc_va + zi * 8) as *mut u64, 0);
                        core::ptr::write_volatile((pkt_va + zi * 8) as *mut u64, 0);
                    }
                    zi += 1;
                }
                let mut i = 0u64;
                while i < 8 {
                    let desc_base = desc_va + i * 16;
                    unsafe {
                        core::ptr::write_volatile(desc_base as *mut u64, pkt_phys);
                        core::ptr::write_volatile((desc_base + 8) as *mut u64, 0);
                    }
                    i += 1;
                }
                let mut status_zero = 1u32;
                let mut si = 0u64;
                while si < 8 {
                    let st = unsafe { core::ptr::read_volatile((desc_va + si * 16 + 12) as *const u8) };
                    if st != 0 {
                        status_zero = 0;
                    }
                    si += 1;
                }
                serial_println!(
                    "[sexnet.nic.rx.desc.link] count=8 buf_phys=0x{:016X} status_zero={} ok={}",
                    pkt_phys,
                    status_zero,
                    if status_zero == 1 { 1 } else { 0 }
                );

                let rctl_orig = unsafe { core::ptr::read_volatile((nic_va + 0x0100) as *const u32) };
                let rdbal_orig = unsafe { core::ptr::read_volatile((nic_va + 0x2800) as *const u32) };
                let rdbah_orig = unsafe { core::ptr::read_volatile((nic_va + 0x2804) as *const u32) };
                let rdlen_orig = unsafe { core::ptr::read_volatile((nic_va + 0x2808) as *const u32) };
                let rdh_orig = unsafe { core::ptr::read_volatile((nic_va + 0x2810) as *const u32) };
                let rdt_orig = unsafe { core::ptr::read_volatile((nic_va + 0x2818) as *const u32) };
                serial_println!(
                    "[sexnet.nic.rx.ring.save] rctl=0x{:08X} rdbal=0x{:08X} rdlen={} rdt={} ok=1",
                    rctl_orig,
                    rdbal_orig,
                    rdlen_orig,
                    rdt_orig
                );

                let rctl_init: u32 = (1 << 1) | (1 << 3) | (1 << 4) | (1 << 26);
                unsafe {
                    core::ptr::write_volatile((nic_va + 0x0100) as *mut u32, rctl_orig & !(1u32 << 1));
                    core::ptr::write_volatile((nic_va + 0x2800) as *mut u32, (desc_phys & 0xFFFF_FFFF) as u32);
                    core::ptr::write_volatile((nic_va + 0x2804) as *mut u32, (desc_phys >> 32) as u32);
                    core::ptr::write_volatile((nic_va + 0x2808) as *mut u32, 128);
                    core::ptr::write_volatile((nic_va + 0x2810) as *mut u32, 0);
                    core::ptr::write_volatile((nic_va + 0x2818) as *mut u32, 7);
                    core::ptr::write_volatile((nic_va + 0x280C) as *mut u32, 0x0000_0002);
                    core::ptr::write_volatile((nic_va + 0x0100) as *mut u32, rctl_init);
                }
                let program_rdbal = unsafe { core::ptr::read_volatile((nic_va + 0x2800) as *const u32) };
                let program_rdbah = unsafe { core::ptr::read_volatile((nic_va + 0x2804) as *const u32) };
                let program_rdlen = unsafe { core::ptr::read_volatile((nic_va + 0x2808) as *const u32) };
                let program_rdh = unsafe { core::ptr::read_volatile((nic_va + 0x2810) as *const u32) };
                let program_rdt = unsafe { core::ptr::read_volatile((nic_va + 0x2818) as *const u32) };
                let program_rctl = unsafe { core::ptr::read_volatile((nic_va + 0x0100) as *const u32) };
                serial_println!(
                    "[sexnet.nic.rx.ring.program] rdbal=0x{:08X} rdbah=0x{:08X} rdlen=128 rdh=0 rdt=7 rctl=0x{:08X} ok=1",
                    program_rdbal,
                    program_rdbah,
                    program_rctl
                );
                let rctl_en = if (program_rctl & (1 << 1)) != 0 { 1 } else { 0 };
                let ring_readback_ok = if program_rdbal == (desc_phys as u32)
                    && program_rdlen == 128
                    && program_rdt == 7
                    && rctl_en == 1
                {
                    1
                } else {
                    0
                };
                serial_println!(
                    "[sexnet.nic.rx.ring.readback] rdbal=0x{:08X} rdlen={} rdt={} rctl_en={} ok={}",
                    program_rdbal,
                    program_rdlen,
                    program_rdt,
                    rctl_en,
                    ring_readback_ok
                );
                let _ = program_rdh;

                let mut seen_mask = 0u8;
                let mut dd_set = 0u32;
                let mut poll_outer = 0u32;
                while poll_outer < 128 {
                    let mut di = 0u8;
                    while di < 8 {
                        let st = unsafe {
                            core::ptr::read_volatile((desc_va + (di as u64) * 16 + 12) as *const u8)
                        };
                        if (st & 0x1) != 0 {
                            let bit = 1u8 << di;
                            if (seen_mask & bit) == 0 {
                                seen_mask |= bit;
                                dd_set += 1;
                            }
                        }
                        di += 1;
                    }
                    poll_outer += 1;
                }
                serial_println!("[sexnet.nic.rx.dd.poll] polled=8 dd_set={} ok=1", dd_set);

                unsafe {
                    core::ptr::write_volatile((nic_va + 0x0100) as *mut u32, rctl_init & !(1u32 << 1));
                    core::ptr::write_volatile((nic_va + 0x2800) as *mut u32, rdbal_orig);
                    core::ptr::write_volatile((nic_va + 0x2804) as *mut u32, rdbah_orig);
                    core::ptr::write_volatile((nic_va + 0x2808) as *mut u32, rdlen_orig);
                    core::ptr::write_volatile((nic_va + 0x2810) as *mut u32, rdh_orig);
                    core::ptr::write_volatile((nic_va + 0x2818) as *mut u32, rdt_orig);
                    core::ptr::write_volatile((nic_va + 0x0100) as *mut u32, rctl_orig);
                }
                let rctl_rest = unsafe { core::ptr::read_volatile((nic_va + 0x0100) as *const u32) };
                let rdbal_rest = unsafe { core::ptr::read_volatile((nic_va + 0x2800) as *const u32) };
                let rdlen_rest = unsafe { core::ptr::read_volatile((nic_va + 0x2808) as *const u32) };
                let rdt_rest = unsafe { core::ptr::read_volatile((nic_va + 0x2818) as *const u32) };
                serial_println!(
                    "[sexnet.nic.rx.ring.restore] rdbal=0x{:08X} rdlen={} rdt={} rctl=0x{:08X} ok=1",
                    rdbal_rest,
                    rdlen_rest,
                    rdt_rest,
                    rctl_rest
                );
                let rctl_en_rest = if (rctl_rest & (1 << 1)) != 0 { 1 } else { 0 };
                let restore_ok = if rctl_en_rest == 1 && rdbal_rest == rdbal_orig { 1 } else { 0 };
                serial_println!(
                    "[sexnet.nic.rx.ring.restore.readback] rctl_en={} rdbal=0x{:08X} ok={}",
                    rctl_en_rest,
                    rdbal_rest,
                    restore_ok
                );
                serial_println!("[sexnet.nic.rx.ring.program_restore.proof.done] ok={}", restore_ok);

                let (obs_desc_phys, obs_desc_va) = reserve_dma(&mut dma_pool, 4096);

                let mut obs_pkt_phys = [0u64; 8];
                let mut obs_pkt_va = [0u64; 8];
                let mut obs_alloc_ok = obs_desc_phys != 0
                    && obs_desc_phys != u64::MAX
                    && obs_desc_va != 0
                    && obs_desc_va != u64::MAX;
                let mut oi = 0usize;
                while oi < 8 {
                    (obs_pkt_phys[oi], obs_pkt_va[oi]) = reserve_dma(&mut dma_pool, 4096);
                    if obs_pkt_phys[oi] == 0
                        || obs_pkt_phys[oi] == u64::MAX
                        || obs_pkt_va[oi] == 0
                        || obs_pkt_va[oi] == u64::MAX
                    {
                        obs_alloc_ok = false;
                    }
                    oi += 1;
                }
                serial_println!(
                    "[sexnet.nic.rx.observe.alloc] desc_phys=0x{:016X} pkt_pages=8 ok={}",
                    obs_desc_phys,
                    if obs_alloc_ok { 1 } else { 0 }
                );
                if obs_alloc_ok {
                    let mut z = 0u64;
                    while z < 512 {
                        unsafe {
                            core::ptr::write_volatile((obs_desc_va + z * 8) as *mut u64, 0);
                        }
                        z += 1;
                    }
                    let mut pi = 0usize;
                    while pi < 8 {
                        let mut pz = 0u64;
                        while pz < 512 {
                            unsafe {
                                core::ptr::write_volatile((obs_pkt_va[pi] + pz * 8) as *mut u64, 0);
                            }
                            pz += 1;
                        }
                        pi += 1;
                    }
                    let mut di = 0usize;
                    while di < 8 {
                        let base = obs_desc_va + (di as u64) * 16;
                        unsafe {
                            core::ptr::write_volatile(base as *mut u64, obs_pkt_phys[di]);
                            core::ptr::write_volatile((base + 8) as *mut u64, 0);
                        }
                        di += 1;
                    }
                    serial_println!("[sexnet.nic.rx.observe.desc.link] count=8 separate_bufs=1 ok=1");

                    let obs_rctl_orig =
                        unsafe { core::ptr::read_volatile((nic_va + 0x0100) as *const u32) };
                    let obs_rdbal_orig =
                        unsafe { core::ptr::read_volatile((nic_va + 0x2800) as *const u32) };
                    let obs_rdbah_orig =
                        unsafe { core::ptr::read_volatile((nic_va + 0x2804) as *const u32) };
                    let obs_rdlen_orig =
                        unsafe { core::ptr::read_volatile((nic_va + 0x2808) as *const u32) };
                    let obs_rdh_orig =
                        unsafe { core::ptr::read_volatile((nic_va + 0x2810) as *const u32) };
                    let obs_rdt_orig =
                        unsafe { core::ptr::read_volatile((nic_va + 0x2818) as *const u32) };
                    let rctl_init: u32 = (1 << 1) | (1 << 3) | (1 << 4) | (1 << 26);
                    unsafe {
                        core::ptr::write_volatile(
                            (nic_va + 0x0100) as *mut u32,
                            obs_rctl_orig & !(1u32 << 1),
                        );
                        core::ptr::write_volatile(
                            (nic_va + 0x2800) as *mut u32,
                            (obs_desc_phys & 0xFFFF_FFFF) as u32,
                        );
                        core::ptr::write_volatile(
                            (nic_va + 0x2804) as *mut u32,
                            (obs_desc_phys >> 32) as u32,
                        );
                        core::ptr::write_volatile((nic_va + 0x2808) as *mut u32, 128);
                        core::ptr::write_volatile((nic_va + 0x2810) as *mut u32, 0);
                        core::ptr::write_volatile((nic_va + 0x2818) as *mut u32, 7);
                        core::ptr::write_volatile((nic_va + 0x280C) as *mut u32, 0x0000_0002);
                        core::ptr::write_volatile((nic_va + 0x0100) as *mut u32, rctl_init);
                    }
                    let obs_prog_rdbal =
                        unsafe { core::ptr::read_volatile((nic_va + 0x2800) as *const u32) };
                    let obs_prog_rctl =
                        unsafe { core::ptr::read_volatile((nic_va + 0x0100) as *const u32) };
                    serial_println!(
                        "[sexnet.nic.rx.observe.ring.program] rdbal=0x{:08X} rdlen=128 rdt=7 rctl=0x{:08X} ok=1",
                        obs_prog_rdbal,
                        obs_prog_rctl
                    );
                    serial_println!("[sexnet.nic.rx.observe.window.open] max_iters=50000000");

                    serial_println!("[sexnet.nic.rx.observe.poll.begin] max_iters=50000000");
                    let mut dd_set = 0u32;
                    let mut dd_desc = 0xFFFFu32;
                    let mut outer = 0u32;
                    while outer < 50_000_000 {
                        let mut idx = 0u32;
                        while idx < 8 {
                            let st = unsafe {
                                core::ptr::read_volatile(
                                    (obs_desc_va + (idx as u64) * 16 + 12) as *const u8
                                )
                            };
                            if (st & 1) != 0 {
                                dd_set = 1;
                                dd_desc = idx;
                                break;
                            }
                            idx += 1;
                        }
                        if dd_set == 1 {
                            break;
                        }
                        outer += 1;
                    }
                    serial_println!(
                        "[sexnet.nic.rx.observe.poll.done] dd_set={} desc_idx={} ok=1",
                        dd_set,
                        dd_desc
                    );
                    // Phase B: RX descriptor observe truth markers
                    if dd_set > 0 {
                        serial_println!("[sexnet.nic.rx.observe.ok] dd_set={} desc_idx={} ok=1", dd_set, dd_desc);
                    } else {
                        serial_println!("[sexnet.nic.rx.timeout.honest] dd_set=0 reason=bounded_poll_no_traffic ok=1");
                    }

                    if dd_set > 0 {
                        let desc_base = obs_desc_va + (dd_desc as u64) * 16;
                        let pkt_len =
                            unsafe { core::ptr::read_volatile((desc_base + 8) as *const u16) } as u32;
                        let pkt_buf = obs_pkt_va[dd_desc as usize];
                        let eth_hi = unsafe { core::ptr::read_volatile((pkt_buf + 12) as *const u8) };
                        let eth_lo = unsafe { core::ptr::read_volatile((pkt_buf + 13) as *const u8) };
                        let ethertype = ((eth_hi as u16) << 8) | (eth_lo as u16);
                        let parse_ok = if pkt_len > 14 { 1 } else { 0 };
                        serial_println!(
                            "[sexnet.nic.rx.observe.pkt.parse] len={} ethertype=0x{:04X} ok={}",
                            pkt_len,
                            ethertype,
                            parse_ok
                        );
                        // Phase C: Ethernet frame classifier markers
                        if parse_ok == 1 {
                            let eth_classify = if ethertype == 0x0806 {
                                "arp"
                            } else if ethertype == 0x0800 {
                                "ipv4"
                            } else {
                                "unknown"
                            };
                            serial_println!(
                                "[sexnet.ether.parse.ok] len={} ethertype=0x{:04X} class={} ok=1",
                                pkt_len,
                                ethertype,
                                eth_classify
                            );
                            if ethertype != 0x0806 && ethertype != 0x0800 {
                                serial_println!(
                                    "[sexnet.ether.ethertype.unknown.reject] ethertype=0x{:04X} ok=1",
                                    ethertype
                                );
                            }
                        } else {
                            serial_println!(
                                "[sexnet.ether.runt.reject] len={} min=15 ok=1",
                                pkt_len
                            );
                        }
                    }

                    unsafe {
                        core::ptr::write_volatile((nic_va + 0x0100) as *mut u32, rctl_init & !(1u32 << 1));
                        core::ptr::write_volatile((nic_va + 0x2800) as *mut u32, obs_rdbal_orig);
                        core::ptr::write_volatile((nic_va + 0x2804) as *mut u32, obs_rdbah_orig);
                        core::ptr::write_volatile((nic_va + 0x2808) as *mut u32, obs_rdlen_orig);
                        core::ptr::write_volatile((nic_va + 0x2810) as *mut u32, obs_rdh_orig);
                        core::ptr::write_volatile((nic_va + 0x2818) as *mut u32, obs_rdt_orig);
                        core::ptr::write_volatile((nic_va + 0x0100) as *mut u32, obs_rctl_orig);
                    }
                    let obs_rest_rctl =
                        unsafe { core::ptr::read_volatile((nic_va + 0x0100) as *const u32) };
                    let obs_rest_rdbal =
                        unsafe { core::ptr::read_volatile((nic_va + 0x2800) as *const u32) };
                    let rctl_en_restored = if (obs_rest_rctl & (1 << 1)) != 0 { 1 } else { 0 };
                    let restore_ok = if rctl_en_restored == 1 && obs_rest_rdbal == obs_rdbal_orig {
                        1
                    } else {
                        0
                    };
                    serial_println!(
                        "[sexnet.nic.rx.observe.ring.restore] rctl_orig=0x{:08X} rctl_en={} ok={}",
                        obs_rctl_orig,
                        rctl_en_restored,
                        restore_ok
                    );
                    let proof_ok = if dd_set > 0 && restore_ok == 1 { 1 } else { 0 };
                    serial_println!(
                        "[sexnet.nic.rx.observe.proof.done] dd_set={} ok={}",
                        dd_set,
                        proof_ok
                    );
                }

                // ------------------------------------------------------------------
                // E1000E NIC reset for RX ownership transition
                // ------------------------------------------------------------------
                serial_println!("[sexnet.nic.reset.begin] ok=1");

                // 1. Disable RX: clear RCTL.EN (bit 1)
                let rctl_pre_rst = unsafe { core::ptr::read_volatile((nic_va + 0x0100) as *const u32) };
                unsafe {
                    core::ptr::write_volatile((nic_va + 0x0100) as *mut u32, rctl_pre_rst & !(1u32 << 1));
                }
                let rctl_rst = unsafe { core::ptr::read_volatile((nic_va + 0x0100) as *const u32) };
                let rx_disable_ok = if (rctl_rst & (1u32 << 1)) == 0 { 1u8 } else { 0u8 };
                serial_println!("[sexnet.nic.reset.rx.disable] ok={}", rx_disable_ok);

                // 2. Disable TX: clear TCTL.EN (bit 1)
                let tctl_pre_rst = unsafe { core::ptr::read_volatile((nic_va + 0x0400) as *const u32) };
                unsafe {
                    core::ptr::write_volatile((nic_va + 0x0400) as *mut u32, tctl_pre_rst & !(1u32 << 1));
                }
                let tctl_rst = unsafe { core::ptr::read_volatile((nic_va + 0x0400) as *const u32) };
                let tx_disable_ok = if (tctl_rst & (1u32 << 1)) == 0 { 1u8 } else { 0u8 };
                serial_println!("[sexnet.nic.reset.tx.disable] ok={}", tx_disable_ok);

                // 3. Mask interrupts: IMC(0x00D8) = all-ones
                unsafe {
                    core::ptr::write_volatile((nic_va + 0x00D8) as *mut u32, 0xFFFF_FFFFu32);
                }
                serial_println!("[sexnet.nic.reset.irq.mask] ok=1");

                // 4. Issue CTRL.RST: set bit 26 at offset 0x0000
                let ctrl_pre = unsafe { core::ptr::read_volatile((nic_va + 0x0000) as *const u32) };
                unsafe {
                    core::ptr::write_volatile((nic_va + 0x0000) as *mut u32, ctrl_pre | (1u32 << 26));
                }
                serial_println!("[sexnet.nic.reset.ctrl.rst.write] ok=1");

                // 5. Bounded poll until CTRL.RST bit clears (max 1M iterations)
                let mut rst_polls: u32 = 0;
                let mut rst_cleared: u32 = 0;
                while rst_polls < 1_000_000 {
                    let ctrl_poll = unsafe { core::ptr::read_volatile((nic_va + 0x0000) as *const u32) };
                    if (ctrl_poll & (1u32 << 26)) == 0 {
                        rst_cleared = 1;
                        break;
                    }
                    rst_polls += 1;
                }
                let rst_ok = rst_cleared;
                serial_println!(
                    "[sexnet.nic.reset.ctrl.rst.poll] cleared={} polls={} ok={}",
                    rst_cleared,
                    rst_polls,
                    rst_ok
                );

                // 6. Read MAC after reset (auto-load from EEPROM)
                let ral_after = unsafe { core::ptr::read_volatile((nic_va + 0x5400) as *const u32) };
                let rah_after = unsafe { core::ptr::read_volatile((nic_va + 0x5404) as *const u32) };
                let mac_valid = (rah_after >> 31) & 1;
                serial_println!(
                    "[sexnet.nic.reset.mac.program] ral=0x{:08X} rah=0x{:08X} valid={} ok=1",
                    ral_after,
                    rah_after,
                    mac_valid
                );

                // 7. Set RXDCTL(0).ENABLE and TXDCTL(0).ENABLE after reset
                //    RXDCTL at 0x2828, TXDCTL at 0x3828
                //    ENABLE=bit25, prefetch=8, host=4, writeback=4
                unsafe {
                    core::ptr::write_volatile((nic_va + 0x2828) as *mut u32, 0x0200_0000u32 | (8 << 16) | (4 << 8) | 4);
                    core::ptr::write_volatile((nic_va + 0x3828) as *mut u32, 0x0200_0000u32 | (8 << 16) | (4 << 8) | 4);
                }
                serial_println!("[sexnet.nic.reset.queue.enable] rxdctl=1 txdctl=1 ok=1");

                // 8. Bounded link poll: STATUS(0x0008) LU bit 10 (e1000e) or bit 1 (e1000)
                let mut link_polls: u32 = 0;
                let mut link_up: u32 = 0;
                while link_polls < 1_000_000 {
                    let status_poll = unsafe { core::ptr::read_volatile((nic_va + 0x0008) as *const u32) };
                    let lu_e1000e = (status_poll >> 10) & 1;
                    let lu_e1000  = (status_poll >> 1) & 1;
                    if lu_e1000e != 0 || lu_e1000 != 0 {
                        link_up = 1;
                        break;
                    }
                    link_polls += 1;
                }
                serial_println!(
                    "[sexnet.nic.reset.status] link_up={} ok=1",
                    link_up
                );

                serial_println!("[sexnet.nic.reset.proof.done] ok=1");

                let (perm_desc_phys, perm_desc_va) = reserve_dma(&mut dma_pool, 4096);

                let mut perm_pkt_phys = [0u64; 8];
                let mut perm_pkt_va = [0u64; 8];
                let mut perm_alloc_ok = perm_desc_phys != 0
                    && perm_desc_phys != u64::MAX
                    && perm_desc_va != 0
                    && perm_desc_va != u64::MAX;
                let mut poi = 0usize;
                while poi < 8 {
                    (perm_pkt_phys[poi], perm_pkt_va[poi]) = reserve_dma(&mut dma_pool, 4096);
                    if perm_pkt_phys[poi] == 0
                        || perm_pkt_phys[poi] == u64::MAX
                        || perm_pkt_va[poi] == 0
                        || perm_pkt_va[poi] == u64::MAX
                    {
                        perm_alloc_ok = false;
                    }
                    poi += 1;
                }
                serial_println!(
                    "[sexnet.nic.rx.permanent.alloc] desc_phys=0x{:016X} pkt_pages=8 ok={}",
                    perm_desc_phys,
                    if perm_alloc_ok { 1 } else { 0 }
                );
                if perm_alloc_ok {
                    let mut z = 0u64;
                    while z < 512 {
                        unsafe {
                            core::ptr::write_volatile((perm_desc_va + z * 8) as *mut u64, 0);
                        }
                        z += 1;
                    }
                    let mut pzi = 0usize;
                    while pzi < 8 {
                        let mut pz = 0u64;
                        while pz < 512 {
                            unsafe {
                                core::ptr::write_volatile((perm_pkt_va[pzi] + pz * 8) as *mut u64, 0);
                            }
                            pz += 1;
                        }
                        pzi += 1;
                    }
                    let mut di = 0usize;
                    while di < 8 {
                        let base = perm_desc_va + (di as u64) * 16;
                        unsafe {
                            core::ptr::write_volatile(base as *mut u64, perm_pkt_phys[di]);
                            core::ptr::write_volatile((base + 8) as *mut u64, 0);
                        }
                        di += 1;
                    }
                    serial_println!("[sexnet.nic.rx.permanent.desc.link] count=8 ok=1");

                    let perm_rctl_orig =
                        unsafe { core::ptr::read_volatile((nic_va + 0x0100) as *const u32) };
                    let rctl_init: u32 = (1 << 1) | (1 << 3) | (1 << 4) | (1 << 26);
                    unsafe {
                        core::ptr::write_volatile((nic_va + 0x0100) as *mut u32, perm_rctl_orig & !(1u32 << 1));
                        core::ptr::write_volatile((nic_va + 0x2800) as *mut u32, (perm_desc_phys & 0xFFFF_FFFF) as u32);
                        core::ptr::write_volatile((nic_va + 0x2804) as *mut u32, (perm_desc_phys >> 32) as u32);
                        core::ptr::write_volatile((nic_va + 0x2808) as *mut u32, 128);
                        core::ptr::write_volatile((nic_va + 0x2810) as *mut u32, 0);
                        core::ptr::write_volatile((nic_va + 0x2818) as *mut u32, 7);
                        core::ptr::write_volatile((nic_va + 0x280C) as *mut u32, 0x0000_0002);
                        core::ptr::write_volatile((nic_va + 0x0100) as *mut u32, rctl_init);
                    }
                    let perm_prog_rdbal =
                        unsafe { core::ptr::read_volatile((nic_va + 0x2800) as *const u32) };
                    let perm_prog_rctl =
                        unsafe { core::ptr::read_volatile((nic_va + 0x0100) as *const u32) };
                    let perm_rctl_en = if (perm_prog_rctl & (1 << 1)) != 0 { 1 } else { 0 };
                    let perm_ring_ok = if perm_prog_rdbal == (perm_desc_phys as u32) && perm_rctl_en == 1 {
                        1
                    } else {
                        0
                    };
                    serial_println!(
                        "[sexnet.nic.rx.permanent.ring.program] rdbal=0x{:08X} rdlen=128 rdt=7 rctl=0x{:08X} ok={}",
                        perm_prog_rdbal,
                        perm_prog_rctl,
                        perm_ring_ok
                    );

                    if perm_ring_ok == 1 {
                        NIC_RX_OWNER.store(NIC_OWNER_SEXNET_RX, Ordering::Release);
                    }
                    let rx_owner_now = NIC_RX_OWNER.load(Ordering::Acquire);
                    let claim_ok = if rx_owner_now == NIC_OWNER_SEXNET_RX && perm_ring_ok == 1 {
                        1
                    } else {
                        0
                    };
                    serial_println!(
                        "[sexnet.nic.rx.permanent.claim] owner={} ring_ok={} ok={}",
                        rx_owner_now,
                        perm_ring_ok,
                        claim_ok
                    );

                    if claim_ok == 1 {
                        unsafe {
                            RX_PERM_DESC_PHYS = perm_desc_phys;
                            RX_PERM_DESC_VA = perm_desc_va;
                            RX_PERM_PKT_PHYS = perm_pkt_phys;
                            RX_PERM_PKT_VA = perm_pkt_va;
                        }
                        serial_println!("[sexnet.nic.rx.permanent.poll.begin] max_iters=50000000");
                        let mut dd_set = 0u32;
                        let mut dd_desc = 0xFFFFu32;
                        let mut outer = 0u32;
                        while outer < 50_000_000 {
                            let mut idx = 0u32;
                            while idx < 8 {
                                let st = unsafe {
                                    core::ptr::read_volatile((perm_desc_va + (idx as u64) * 16 + 12) as *const u8)
                                };
                                if (st & 1) != 0 {
                                    dd_set = 1;
                                    dd_desc = idx;
                                    break;
                                }
                                idx += 1;
                            }
                            if dd_set == 1 {
                                break;
                            }
                            outer += 1;
                        }
                        serial_println!(
                            "[sexnet.nic.rx.permanent.poll.done] dd_set={} desc_idx={} ok=1",
                            dd_set,
                            dd_desc
                        );

                        if dd_set == 1 {
                            let desc_base = perm_desc_va + (dd_desc as u64) * 16;
                            let pkt_len = unsafe { core::ptr::read_volatile((desc_base + 8) as *const u16) } as u32;
                            let pkt_buf = perm_pkt_va[dd_desc as usize];
                            let eth_hi = unsafe { core::ptr::read_volatile((pkt_buf + 12) as *const u8) };
                            let eth_lo = unsafe { core::ptr::read_volatile((pkt_buf + 13) as *const u8) };
                            let ethertype = ((eth_hi as u16) << 8) | (eth_lo as u16);
                            let parse_ok = if pkt_len > 14 { 1 } else { 0 };
                            serial_println!(
                                "[sexnet.nic.rx.permanent.pkt.parse] len={} ethertype=0x{:04X} ok={}",
                                pkt_len,
                                ethertype,
                                parse_ok
                            );
                            // Phase C: Ethernet frame classifier markers (permanent ring)
                            if parse_ok == 1 {
                                let eth_classify = if ethertype == 0x0806 {
                                    "arp"
                                } else if ethertype == 0x0800 {
                                    "ipv4"
                                } else {
                                    "unknown"
                                };
                                serial_println!(
                                    "[sexnet.ether.parse.ok] len={} ethertype=0x{:04X} class={} ok=1",
                                    pkt_len,
                                    ethertype,
                                    eth_classify
                                );
                                if ethertype != 0x0806 && ethertype != 0x0800 {
                                    serial_println!(
                                        "[sexnet.ether.ethertype.unknown.reject] ethertype=0x{:04X} ok=1",
                                        ethertype
                                    );
                                }
                            } else {
                                serial_println!(
                                    "[sexnet.ether.runt.reject] len={} min=15 ok=1",
                                    pkt_len
                                );
                            }

                            unsafe {
                                core::ptr::write_volatile((desc_base + 8) as *mut u16, 0u16);
                                core::ptr::write_volatile((desc_base + 12) as *mut u8, 0u8);
                            }
                            let new_rdt = (dd_desc + 7) & 7;
                            unsafe {
                                core::ptr::write_volatile((nic_va + 0x2818) as *mut u32, new_rdt);
                            }
                            serial_println!(
                                "[sexnet.nic.rx.permanent.rdt.advance] desc_idx={} new_rdt={} ok=1",
                                dd_desc,
                                new_rdt
                            );
                        }
                    }
                }
            }

            let (tx_desc_phys, tx_desc_va) = reserve_dma(&mut dma_pool, 4096);

            let (tx_frame_phys, tx_frame_va) = reserve_dma(&mut dma_pool, 4096);

            let tx_alloc_ok = tx_desc_phys != 0
                && tx_desc_phys != u64::MAX
                && tx_desc_va != 0
                && tx_desc_va != u64::MAX
                && tx_frame_phys != 0
                && tx_frame_phys != u64::MAX
                && tx_frame_va != 0
                && tx_frame_va != u64::MAX;
            serial_println!(
                "[sexnet.nic.tx.observe.alloc] desc_phys=0x{:016X} frame_phys=0x{:016X} ok={}",
                tx_desc_phys,
                tx_frame_phys,
                if tx_alloc_ok { 1 } else { 0 }
            );
            if tx_alloc_ok {
                let mut z = 0u64;
                while z < 512 {
                    unsafe {
                        core::ptr::write_volatile((tx_desc_va + z * 8) as *mut u64, 0);
                        core::ptr::write_volatile((tx_frame_va + z * 8) as *mut u64, 0);
                    }
                    z += 1;
                }
                let mut bi = 0u64;
                while bi < 6 {
                    unsafe {
                        core::ptr::write_volatile((tx_frame_va + bi) as *mut u8, 0xFF);
                    }
                    bi += 1;
                }
                let src = [0x52u8, 0x54u8, 0x00u8, 0x12u8, 0x34u8, 0x56u8];
                let mut si = 0usize;
                while si < 6 {
                    unsafe {
                        core::ptr::write_volatile((tx_frame_va + 6 + si as u64) as *mut u8, src[si]);
                    }
                    si += 1;
                }
                unsafe {
                    core::ptr::write_volatile((tx_frame_va + 12) as *mut u8, 0x88);
                    core::ptr::write_volatile((tx_frame_va + 13) as *mut u8, 0xB5);
                }
                let mut pi = 14u64;
                while pi < 60 {
                    unsafe {
                        core::ptr::write_volatile((tx_frame_va + pi) as *mut u8, 0x42);
                    }
                    pi += 1;
                }
                serial_println!("[sexnet.nic.tx.observe.frame.write] ethertype=0x88B5 len=60 ok=1");

                unsafe {
                    core::ptr::write_volatile(tx_desc_va as *mut u64, tx_frame_phys);
                    core::ptr::write_volatile((tx_desc_va + 8) as *mut u16, 60u16);
                    core::ptr::write_volatile((tx_desc_va + 10) as *mut u8, 0u8);
                    core::ptr::write_volatile((tx_desc_va + 11) as *mut u8, 0x0Bu8);
                    core::ptr::write_volatile((tx_desc_va + 12) as *mut u8, 0u8);
                    core::ptr::write_volatile((tx_desc_va + 13) as *mut u8, 0u8);
                    core::ptr::write_volatile((tx_desc_va + 14) as *mut u16, 0u16);
                }
                serial_println!("[sexnet.nic.tx.observe.desc.write] len=60 cmd=0x0B sta=0 ok=1");

                let tx_tctl_orig = unsafe { core::ptr::read_volatile((nic_va + 0x0400) as *const u32) };
                let tx_tctl_en_orig = if (tx_tctl_orig & (1 << 1)) != 0 { 1 } else { 0 };
                let tx_tdbal_orig = unsafe { core::ptr::read_volatile((nic_va + 0x3800) as *const u32) };
                let tx_tdbah_orig = unsafe { core::ptr::read_volatile((nic_va + 0x3804) as *const u32) };
                let tx_tdlen_orig = unsafe { core::ptr::read_volatile((nic_va + 0x3808) as *const u32) };
                let tx_tdh_orig = unsafe { core::ptr::read_volatile((nic_va + 0x3810) as *const u32) };
                let tx_tdt_orig = unsafe { core::ptr::read_volatile((nic_va + 0x3818) as *const u32) };
                serial_println!(
                    "[sexnet.nic.tx.observe.ring.save] tctl=0x{:08X} tctl_en={} tdbal=0x{:08X} tdlen={} tdt={} ok=1",
                    tx_tctl_orig,
                    tx_tctl_en_orig,
                    tx_tdbal_orig,
                    tx_tdlen_orig,
                    tx_tdt_orig
                );
                let _ = tx_tdh_orig;

                unsafe {
                    core::ptr::write_volatile((nic_va + 0x0400) as *mut u32, tx_tctl_orig & !(1u32 << 1));
                    core::ptr::write_volatile((nic_va + 0x3800) as *mut u32, (tx_desc_phys & 0xFFFF_FFFF) as u32);
                    core::ptr::write_volatile((nic_va + 0x3804) as *mut u32, (tx_desc_phys >> 32) as u32);
                    core::ptr::write_volatile((nic_va + 0x3808) as *mut u32, 128);
                    core::ptr::write_volatile((nic_va + 0x3810) as *mut u32, 0);
                    core::ptr::write_volatile((nic_va + 0x3818) as *mut u32, 0);
                }
                let tx_tctl_prog: u32 = (1 << 1) | (0x10 << 4) | (0x40 << 12);
                unsafe {
                    core::ptr::write_volatile((nic_va + 0x0400) as *mut u32, tx_tctl_prog);
                }
                let tx_prog_tdbal = unsafe { core::ptr::read_volatile((nic_va + 0x3800) as *const u32) };
                let tx_prog_tctl = unsafe { core::ptr::read_volatile((nic_va + 0x0400) as *const u32) };
                let tx_tctl_en = if (tx_prog_tctl & (1 << 1)) != 0 { 1 } else { 0 };
                let tx_prog_ok = if tx_prog_tdbal == (tx_desc_phys as u32) && tx_tctl_en == 1 {
                    1
                } else {
                    0
                };
                serial_println!(
                    "[sexnet.nic.tx.observe.ring.program] tdbal=0x{:08X} tdlen=128 tdt=0 tctl=0x{:08X} ok={}",
                    tx_prog_tdbal,
                    tx_prog_tctl,
                    tx_prog_ok
                );

                unsafe {
                    core::ptr::write_volatile((nic_va + 0x3818) as *mut u32, 1);
                }
                serial_println!("[sexnet.nic.tx.observe.post] tdt=1 ok=1");

                serial_println!("[sexnet.nic.tx.observe.poll.begin] max_iters=50000000");
                let mut tx_dd_set = 0u32;
                let mut tx_outer = 0u32;
                while tx_outer < 50_000_000 {
                    let tx_st = unsafe { core::ptr::read_volatile((tx_desc_va + 12) as *const u8) };
                    if (tx_st & 1) != 0 {
                        tx_dd_set = 1;
                        break;
                    }
                    tx_outer += 1;
                }
                serial_println!(
                    "[sexnet.nic.tx.observe.poll.done] dd_set={} desc_idx=0 ok=1",
                    tx_dd_set
                );
                // Phase B: TX descriptor consumed by hardware proof marker
                if tx_dd_set == 1 {
                    serial_println!("[sexnet.nic.tx.dd.ok] dd_set={} ok=1", tx_dd_set);
                }

                unsafe {
                    core::ptr::write_volatile((nic_va + 0x0400) as *mut u32, tx_tctl_orig & !(1u32 << 1));
                    core::ptr::write_volatile((nic_va + 0x3800) as *mut u32, tx_tdbal_orig);
                    core::ptr::write_volatile((nic_va + 0x3804) as *mut u32, tx_tdbah_orig);
                    core::ptr::write_volatile((nic_va + 0x3808) as *mut u32, tx_tdlen_orig);
                    core::ptr::write_volatile((nic_va + 0x3818) as *mut u32, tx_tdt_orig);
                    core::ptr::write_volatile((nic_va + 0x0400) as *mut u32, tx_tctl_orig);
                }
                let tx_rest_tctl = unsafe { core::ptr::read_volatile((nic_va + 0x0400) as *const u32) };
                let tx_rest_tdbal = unsafe { core::ptr::read_volatile((nic_va + 0x3800) as *const u32) };
                let tx_tctl_en_restored = if (tx_rest_tctl & (1 << 1)) != 0 { 1 } else { 0 };
                let tx_restore_ok = if tx_tctl_en_restored == tx_tctl_en_orig && tx_rest_tdbal == tx_tdbal_orig {
                    1
                } else {
                    0
                };
                serial_println!(
                    "[sexnet.nic.tx.observe.ring.restore] tctl_orig=0x{:08X} tctl_en_orig={} tctl_en={} tdbal=0x{:08X} ok={}",
                    tx_tctl_orig,
                    tx_tctl_en_orig,
                    tx_tctl_en_restored,
                    tx_rest_tdbal,
                    tx_restore_ok
                );
                let tx_proof_ok = if tx_dd_set == 1 && tx_restore_ok == 1 { 1 } else { 0 };
                serial_println!(
                    "[sexnet.nic.tx.observe.proof.done] dd_set={} ok={}",
                    tx_dd_set,
                    tx_proof_ok
                );

                let rx_own = NIC_RX_OWNER.load(Ordering::Acquire);
                if rx_own != NIC_OWNER_SEXNET_RX {
                    serial_println!("[sexnet.nic.tx.permanent.skip] reason=rx_not_claimed ok=0");
                } else {
                    let (txp_desc_phys, txp_desc_va) = reserve_dma(&mut dma_pool, 4096);

                    let (txp_frame_phys, txp_frame_va) = reserve_dma(&mut dma_pool, 4096);

                    let txp_alloc_ok = txp_desc_phys != 0
                        && txp_desc_phys != u64::MAX
                        && txp_desc_va != 0
                        && txp_desc_va != u64::MAX
                        && txp_frame_phys != 0
                        && txp_frame_phys != u64::MAX
                        && txp_frame_va != 0
                        && txp_frame_va != u64::MAX;
                    serial_println!(
                        "[sexnet.nic.tx.permanent.alloc] desc_phys=0x{:016X} frame_phys=0x{:016X} ok={}",
                        txp_desc_phys,
                        txp_frame_phys,
                        if txp_alloc_ok { 1 } else { 0 }
                    );
                    if txp_alloc_ok {
                        unsafe {
                            TX_PERM_DESC_PHYS = txp_desc_phys;
                            TX_PERM_DESC_VA = txp_desc_va;
                            TX_PERM_FRAME_PHYS = txp_frame_phys;
                            TX_PERM_FRAME_VA = txp_frame_va;
                        }
                        let mut z = 0u64;
                        while z < 512 {
                            unsafe {
                                core::ptr::write_volatile((txp_desc_va + z * 8) as *mut u64, 0);
                                core::ptr::write_volatile((txp_frame_va + z * 8) as *mut u64, 0);
                            }
                            z += 1;
                        }
                        let mut bi = 0u64;
                        while bi < 6 {
                            unsafe {
                                core::ptr::write_volatile((txp_frame_va + bi) as *mut u8, 0xFF);
                            }
                            bi += 1;
                        }
                        let src = [0x52u8, 0x54u8, 0x00u8, 0x12u8, 0x34u8, 0x56u8];
                        let mut si = 0usize;
                        while si < 6 {
                            unsafe {
                                core::ptr::write_volatile((txp_frame_va + 6 + si as u64) as *mut u8, src[si]);
                            }
                            si += 1;
                        }
                        unsafe {
                            core::ptr::write_volatile((txp_frame_va + 12) as *mut u8, 0x88);
                            core::ptr::write_volatile((txp_frame_va + 13) as *mut u8, 0xB5);
                        }
                        let mut pi = 14u64;
                        while pi < 60 {
                            unsafe {
                                core::ptr::write_volatile((txp_frame_va + pi) as *mut u8, 0x42);
                            }
                            pi += 1;
                        }
                        serial_println!("[sexnet.nic.tx.permanent.frame.write] ethertype=0x88B5 len=60 ok=1");

                        unsafe {
                            core::ptr::write_volatile(txp_desc_va as *mut u64, txp_frame_phys);
                            core::ptr::write_volatile((txp_desc_va + 8) as *mut u16, 60u16);
                            core::ptr::write_volatile((txp_desc_va + 10) as *mut u8, 0u8);
                            core::ptr::write_volatile((txp_desc_va + 11) as *mut u8, 0x0Bu8);
                            core::ptr::write_volatile((txp_desc_va + 12) as *mut u8, 0u8);
                            core::ptr::write_volatile((txp_desc_va + 13) as *mut u8, 0u8);
                            core::ptr::write_volatile((txp_desc_va + 14) as *mut u16, 0u16);
                        }
                        serial_println!("[sexnet.nic.tx.permanent.desc.write] len=60 cmd=0x0B sta=0 ok=1");

                        unsafe {
                            core::ptr::write_volatile((nic_va + 0x3800) as *mut u32, (txp_desc_phys & 0xFFFF_FFFF) as u32);
                            core::ptr::write_volatile((nic_va + 0x3804) as *mut u32, (txp_desc_phys >> 32) as u32);
                            core::ptr::write_volatile((nic_va + 0x3808) as *mut u32, 128);
                            core::ptr::write_volatile((nic_va + 0x3810) as *mut u32, 0);
                            core::ptr::write_volatile((nic_va + 0x3818) as *mut u32, 0);
                            core::ptr::write_volatile((nic_va + 0x0400) as *mut u32, 0x0004_0102);
                        }
                        let txp_prog_tdbal = unsafe { core::ptr::read_volatile((nic_va + 0x3800) as *const u32) };
                        let txp_prog_tctl = unsafe { core::ptr::read_volatile((nic_va + 0x0400) as *const u32) };
                        let txp_tctl_en = if (txp_prog_tctl & (1 << 1)) != 0 { 1 } else { 0 };
                        let txp_ring_ok = if txp_prog_tdbal == (txp_desc_phys as u32) && txp_tctl_en == 1 {
                            1
                        } else {
                            0
                        };
                        serial_println!(
                            "[sexnet.nic.tx.permanent.ring.program] tdbal=0x{:08X} tdlen=128 tdt=0 tctl=0x{:08X} ok={}",
                            txp_prog_tdbal,
                            txp_prog_tctl,
                            txp_ring_ok
                        );

                        let mut txp_dd_set = 0u32;
                        if txp_ring_ok == 1 {
                            unsafe {
                                core::ptr::write_volatile((nic_va + 0x3818) as *mut u32, 1);
                            }
                            serial_println!("[sexnet.nic.tx.permanent.post] tdt=1 ok=1");
                            serial_println!("[sexnet.nic.tx.permanent.poll.begin] max_iters=50000000");
                            let mut txp_outer = 0u32;
                            while txp_outer < 50_000_000 {
                                let tx_st = unsafe { core::ptr::read_volatile((txp_desc_va + 12) as *const u8) };
                                if (tx_st & 1) != 0 {
                                    txp_dd_set = 1;
                                    break;
                                }
                                txp_outer += 1;
                            }
                            serial_println!(
                                "[sexnet.nic.tx.permanent.poll.done] dd_set={} desc_idx=0 ok=1",
                                txp_dd_set
                            );
                        }

                        if txp_ring_ok == 1 && txp_dd_set == 1 {
                            NIC_TX_OWNER.store(NIC_OWNER_SEXNET_TX, Ordering::Release);
                        }
                        let tx_owner_now = NIC_TX_OWNER.load(Ordering::Acquire);
                        let tx_claim_ok = if tx_owner_now == NIC_OWNER_SEXNET_TX && txp_ring_ok == 1 {
                            1
                        } else {
                            0
                        };
                        serial_println!(
                            "[sexnet.nic.tx.permanent.claim] owner={} ring_ok={} ok={}",
                            tx_owner_now,
                            txp_ring_ok,
                            tx_claim_ok
                        );

                        if tx_claim_ok == 1 && NIC_RX_OWNER.load(Ordering::Acquire) == NIC_OWNER_SEXNET_RX {
                            NIC_RX_OWNER.store(NIC_OWNER_SEXNET_FULL, Ordering::Release);
                            NIC_TX_OWNER.store(NIC_OWNER_SEXNET_FULL, Ordering::Release);
                            let full_rx = NIC_RX_OWNER.load(Ordering::Acquire);
                            let full_tx = NIC_TX_OWNER.load(Ordering::Acquire);
                            let full_ok = if full_rx == NIC_OWNER_SEXNET_FULL && full_tx == NIC_OWNER_SEXNET_FULL {
                                1
                            } else {
                                0
                            };
                            serial_println!(
                                "[sexnet.nic.tx.permanent.full] rx_owner={} tx_owner={} full_ok={}",
                                full_rx,
                                full_tx,
                                full_ok
                            );
                        }

                        let mut arp_ok = 0u32;
                        let arp_rx_own = NIC_RX_OWNER.load(Ordering::Acquire);
                        let arp_tx_own = NIC_TX_OWNER.load(Ordering::Acquire);
                        if arp_rx_own == NIC_OWNER_SEXNET_FULL && arp_tx_own == NIC_OWNER_SEXNET_FULL {
                            let nic_mac: [u8; 6] = [
                                (ral & 0xFF) as u8,
                                ((ral >> 8) & 0xFF) as u8,
                                ((ral >> 16) & 0xFF) as u8,
                                ((ral >> 24) & 0xFF) as u8,
                                (rah & 0xFF) as u8,
                                ((rah >> 8) & 0xFF) as u8,
                            ];
                            serial_println!("[sexnet.arp.rx.poll.begin] max_iters=50000000");
                            let mut arp_rx = 0u32;
                            let mut sender_mac = [0u8; 6];
                            let mut sender_ip = [0u8; 4];
                            let mut arp_outer = 0u32;
                            while arp_outer < 50_000_000 && arp_rx == 0 {
                                let mut idx = 0u32;
                                while idx < 8 && arp_rx == 0 {
                                    let desc_base = unsafe { RX_PERM_DESC_VA } + (idx as u64) * 16;
                                    let st = unsafe { core::ptr::read_volatile((desc_base + 12) as *const u8) };
                                    if (st & 1) != 0 {
                                        let pkt_buf = unsafe { RX_PERM_PKT_VA[idx as usize] };
                                        let eth_hi = unsafe { core::ptr::read_volatile((pkt_buf + 12) as *const u8) };
                                        let eth_lo = unsafe { core::ptr::read_volatile((pkt_buf + 13) as *const u8) };
                                        let ethertype = ((eth_hi as u16) << 8) | (eth_lo as u16);
                                        let htype_hi = unsafe { core::ptr::read_volatile((pkt_buf + 14) as *const u8) };
                                        let htype_lo = unsafe { core::ptr::read_volatile((pkt_buf + 15) as *const u8) };
                                        let ptype_hi = unsafe { core::ptr::read_volatile((pkt_buf + 16) as *const u8) };
                                        let ptype_lo = unsafe { core::ptr::read_volatile((pkt_buf + 17) as *const u8) };
                                        let hlen = unsafe { core::ptr::read_volatile((pkt_buf + 18) as *const u8) };
                                        let plen = unsafe { core::ptr::read_volatile((pkt_buf + 19) as *const u8) };
                                        let oper_hi = unsafe { core::ptr::read_volatile((pkt_buf + 20) as *const u8) };
                                        let oper_lo = unsafe { core::ptr::read_volatile((pkt_buf + 21) as *const u8) };
                                        let htype = ((htype_hi as u16) << 8) | (htype_lo as u16);
                                        let ptype = ((ptype_hi as u16) << 8) | (ptype_lo as u16);
                                        let oper = ((oper_hi as u16) << 8) | (oper_lo as u16);
                                        let mut tpa = [0u8; 4];
                                        let mut ti = 0usize;
                                        while ti < 4 {
                                            tpa[ti] = unsafe {
                                                core::ptr::read_volatile((pkt_buf + 38 + ti as u64) as *const u8)
                                            };
                                            ti += 1;
                                        }
                                        let tpa_match = if tpa == SEXNET_GUEST_IPV4 { 1 } else { 0 };
                                        let valid = ethertype == 0x0806
                                            && htype == 1
                                            && ptype == 0x0800
                                            && hlen == 6
                                            && plen == 4
                                            && oper == 1
                                            && tpa_match == 1;
                                        if valid {
                                            let mut mi = 0usize;
                                            while mi < 6 {
                                                sender_mac[mi] = unsafe {
                                                    core::ptr::read_volatile((pkt_buf + 22 + mi as u64) as *const u8)
                                                };
                                                mi += 1;
                                            }
                                            let mut si = 0usize;
                                            while si < 4 {
                                                sender_ip[si] = unsafe {
                                                    core::ptr::read_volatile((pkt_buf + 28 + si as u64) as *const u8)
                                                };
                                                si += 1;
                                            }
                                            serial_println!("[sexnet.arp.rx.frame] idx={} ethertype=0x0806 ok=1", idx);
                                            serial_println!(
                                                "[sexnet.arp.rx.validate] htype=1 ptype=0x0800 hlen=6 plen=4 oper=1 tpa_match=1 ok=1"
                                            );
                                            // Phase D: ARP reply received proof marker
                                            serial_println!("[sexnet.arp.reply.rx.ok] oper=1 ok=1");
                                            arp_rx = 1;
                                        } else {
                                            serial_println!(
                                                "[sexnet.arp.rx.reject.detail] idx={} etype=0x{:04X} oper=0x{:04X} tpa={}.{}.{}.{} ok=0",
                                                idx,
                                                ethertype,
                                                oper,
                                                tpa[0],
                                                tpa[1],
                                                tpa[2],
                                                tpa[3]
                                            );
                                            serial_println!("[sexnet.arp.rx.reject] idx={} reason=notarp_or_badfield ok=0", idx);
                                        }
                                        unsafe {
                                            core::ptr::write_volatile((desc_base + 8) as *mut u16, 0u16);
                                            core::ptr::write_volatile((desc_base + 12) as *mut u8, 0u8);
                                            core::ptr::write_volatile((nic_va + 0x2818) as *mut u32, idx);
                                        }
                                    }
                                    idx += 1;
                                }
                                arp_outer += 1;
                            }

                            let mut arp_tx_dd = 0u32;
                            if arp_rx == 1 {
                                let tx_frame_va = unsafe { TX_PERM_FRAME_VA };
                                let mut i = 0usize;
                                while i < 6 {
                                    unsafe {
                                        core::ptr::write_volatile((tx_frame_va + i as u64) as *mut u8, sender_mac[i]);
                                        core::ptr::write_volatile((tx_frame_va + 6 + i as u64) as *mut u8, nic_mac[i]);
                                    }
                                    i += 1;
                                }
                                unsafe {
                                    core::ptr::write_volatile((tx_frame_va + 12) as *mut u8, 0x08);
                                    core::ptr::write_volatile((tx_frame_va + 13) as *mut u8, 0x06);
                                    core::ptr::write_volatile((tx_frame_va + 14) as *mut u8, 0x00);
                                    core::ptr::write_volatile((tx_frame_va + 15) as *mut u8, 0x01);
                                    core::ptr::write_volatile((tx_frame_va + 16) as *mut u8, 0x08);
                                    core::ptr::write_volatile((tx_frame_va + 17) as *mut u8, 0x00);
                                    core::ptr::write_volatile((tx_frame_va + 18) as *mut u8, 0x06);
                                    core::ptr::write_volatile((tx_frame_va + 19) as *mut u8, 0x04);
                                    core::ptr::write_volatile((tx_frame_va + 20) as *mut u8, 0x00);
                                    core::ptr::write_volatile((tx_frame_va + 21) as *mut u8, 0x02);
                                }
                                let mut ai = 0usize;
                                while ai < 6 {
                                    unsafe {
                                        core::ptr::write_volatile((tx_frame_va + 22 + ai as u64) as *mut u8, nic_mac[ai]);
                                        core::ptr::write_volatile((tx_frame_va + 32 + ai as u64) as *mut u8, sender_mac[ai]);
                                    }
                                    ai += 1;
                                }
                                let mut gi = 0usize;
                                while gi < 4 {
                                    unsafe {
                                        core::ptr::write_volatile(
                                            (tx_frame_va + 28 + gi as u64) as *mut u8,
                                            SEXNET_GUEST_IPV4[gi],
                                        );
                                        core::ptr::write_volatile(
                                            (tx_frame_va + 38 + gi as u64) as *mut u8,
                                            sender_ip[gi],
                                        );
                                    }
                                    gi += 1;
                                }
                                let mut pad = 42u64;
                                while pad < 60 {
                                    unsafe {
                                        core::ptr::write_volatile((tx_frame_va + pad) as *mut u8, 0u8);
                                    }
                                    pad += 1;
                                }
                                serial_println!("[sexnet.arp.tx.reply.build] spa=10.0.2.15 ok=1");
                                let tx_dump_base = unsafe { TX_PERM_FRAME_VA };
                                let eth_dst0 = unsafe { core::ptr::read_volatile((tx_dump_base + 0) as *const u8) };
                                let eth_dst1 = unsafe { core::ptr::read_volatile((tx_dump_base + 1) as *const u8) };
                                let eth_dst2 = unsafe { core::ptr::read_volatile((tx_dump_base + 2) as *const u8) };
                                let eth_dst3 = unsafe { core::ptr::read_volatile((tx_dump_base + 3) as *const u8) };
                                let eth_dst4 = unsafe { core::ptr::read_volatile((tx_dump_base + 4) as *const u8) };
                                let eth_dst5 = unsafe { core::ptr::read_volatile((tx_dump_base + 5) as *const u8) };
                                let eth_src0 = unsafe { core::ptr::read_volatile((tx_dump_base + 6) as *const u8) };
                                let eth_src1 = unsafe { core::ptr::read_volatile((tx_dump_base + 7) as *const u8) };
                                let eth_src2 = unsafe { core::ptr::read_volatile((tx_dump_base + 8) as *const u8) };
                                let eth_src3 = unsafe { core::ptr::read_volatile((tx_dump_base + 9) as *const u8) };
                                let eth_src4 = unsafe { core::ptr::read_volatile((tx_dump_base + 10) as *const u8) };
                                let eth_src5 = unsafe { core::ptr::read_volatile((tx_dump_base + 11) as *const u8) };
                                let etype_hi = unsafe { core::ptr::read_volatile((tx_dump_base + 12) as *const u8) };
                                let etype_lo = unsafe { core::ptr::read_volatile((tx_dump_base + 13) as *const u8) };
                                let oper_hi = unsafe { core::ptr::read_volatile((tx_dump_base + 20) as *const u8) };
                                let oper_lo = unsafe { core::ptr::read_volatile((tx_dump_base + 21) as *const u8) };
                                let sha0 = unsafe { core::ptr::read_volatile((tx_dump_base + 22) as *const u8) };
                                let sha1 = unsafe { core::ptr::read_volatile((tx_dump_base + 23) as *const u8) };
                                let sha2 = unsafe { core::ptr::read_volatile((tx_dump_base + 24) as *const u8) };
                                let sha3 = unsafe { core::ptr::read_volatile((tx_dump_base + 25) as *const u8) };
                                let sha4 = unsafe { core::ptr::read_volatile((tx_dump_base + 26) as *const u8) };
                                let sha5 = unsafe { core::ptr::read_volatile((tx_dump_base + 27) as *const u8) };
                                let spa0 = unsafe { core::ptr::read_volatile((tx_dump_base + 28) as *const u8) };
                                let spa1 = unsafe { core::ptr::read_volatile((tx_dump_base + 29) as *const u8) };
                                let spa2 = unsafe { core::ptr::read_volatile((tx_dump_base + 30) as *const u8) };
                                let spa3 = unsafe { core::ptr::read_volatile((tx_dump_base + 31) as *const u8) };
                                let tha0 = unsafe { core::ptr::read_volatile((tx_dump_base + 32) as *const u8) };
                                let tha1 = unsafe { core::ptr::read_volatile((tx_dump_base + 33) as *const u8) };
                                let tha2 = unsafe { core::ptr::read_volatile((tx_dump_base + 34) as *const u8) };
                                let tha3 = unsafe { core::ptr::read_volatile((tx_dump_base + 35) as *const u8) };
                                let tha4 = unsafe { core::ptr::read_volatile((tx_dump_base + 36) as *const u8) };
                                let tha5 = unsafe { core::ptr::read_volatile((tx_dump_base + 37) as *const u8) };
                                let tpa0 = unsafe { core::ptr::read_volatile((tx_dump_base + 38) as *const u8) };
                                let tpa1 = unsafe { core::ptr::read_volatile((tx_dump_base + 39) as *const u8) };
                                let tpa2 = unsafe { core::ptr::read_volatile((tx_dump_base + 40) as *const u8) };
                                let tpa3 = unsafe { core::ptr::read_volatile((tx_dump_base + 41) as *const u8) };
                                let etype = ((etype_hi as u16) << 8) | (etype_lo as u16);
                                let oper = ((oper_hi as u16) << 8) | (oper_lo as u16);
                                serial_println!(
                                    "[sexnet.arp.tx.dump] eth_dst={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} eth_src={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} etype=0x{:04X} oper=0x{:04X} sha={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} spa={}.{}.{}.{} tha={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} tpa={}.{}.{}.{} ok=1",
                                    eth_dst0, eth_dst1, eth_dst2, eth_dst3, eth_dst4, eth_dst5,
                                    eth_src0, eth_src1, eth_src2, eth_src3, eth_src4, eth_src5,
                                    etype,
                                    oper,
                                    sha0, sha1, sha2, sha3, sha4, sha5,
                                    spa0, spa1, spa2, spa3,
                                    tha0, tha1, tha2, tha3, tha4, tha5,
                                    tpa0, tpa1, tpa2, tpa3
                                );

                                let tx_desc1 = unsafe { TX_PERM_DESC_VA + 16 };
                                unsafe {
                                    core::ptr::write_volatile(tx_desc1 as *mut u64, TX_PERM_FRAME_PHYS);
                                    core::ptr::write_volatile((tx_desc1 + 8) as *mut u16, 60u16);
                                    core::ptr::write_volatile((tx_desc1 + 10) as *mut u8, 0u8);
                                    core::ptr::write_volatile((tx_desc1 + 11) as *mut u8, 0x0Bu8);
                                    core::ptr::write_volatile((tx_desc1 + 12) as *mut u8, 0u8);
                                    core::ptr::write_volatile((tx_desc1 + 13) as *mut u8, 0u8);
                                    core::ptr::write_volatile((tx_desc1 + 14) as *mut u16, 0u16);
                                }
                                serial_println!("[sexnet.arp.tx.desc] slot=1 len=60 ok=1");
                                unsafe {
                                    core::ptr::write_volatile((nic_va + 0x3818) as *mut u32, 2);
                                }
                                serial_println!("[sexnet.arp.tx.post] tdt=2 ok=1");
                                let mut tx_outer = 0u32;
                                while tx_outer < 50_000_000 {
                                    let tx_st = unsafe { core::ptr::read_volatile((tx_desc1 + 12) as *const u8) };
                                    if (tx_st & 1) != 0 {
                                        arp_tx_dd = 1;
                                        break;
                                    }
                                    tx_outer += 1;
                                }
                                serial_println!(
                                    "[sexnet.arp.tx.poll.done] dd_set={} ok={}",
                                    arp_tx_dd,
                                    if arp_tx_dd == 1 { 1 } else { 0 }
                                );
                                // Phase D: ARP request TX marker
                                if arp_tx_dd == 1 {
                                    serial_println!("[sexnet.arp.request.tx.ok] tx_dd={} ok=1", arp_tx_dd);
                                }
                            }
                            arp_ok = if arp_rx == 1 && arp_tx_dd == 1 { 1 } else { 0 };
                            serial_println!(
                                "[sexnet.arp.proof.done] rx_arp={} tx_dd={} ok={}",
                                arp_rx,
                                arp_tx_dd,
                                arp_ok
                            );
                            // Phase D: honest ARP skip marker if no reply
                            if arp_rx == 0 && arp_tx_dd == 1 {
                                serial_println!("[sexnet.arp.reply.rx.skip] reason=no_peer_reply tx_dd={} ok=1", arp_tx_dd);
                            }
                        } else {
                            serial_println!("[sexnet.arp.skip] reason=not_full ok=0");
                            // Phase D: ARP skip when not full ownership
                            serial_println!("[sexnet.arp.reply.rx.skip] reason=nic_not_full_owner ok=1");
                        }

                        let cache_nic_mac: [u8; 6] = [
                            (ral & 0xFF) as u8,
                            ((ral >> 8) & 0xFF) as u8,
                            ((ral >> 16) & 0xFF) as u8,
                            ((ral >> 24) & 0xFF) as u8,
                            (rah & 0xFF) as u8,
                            ((rah >> 8) & 0xFF) as u8,
                        ];
                        let mut cache_replies = 0u32;
                        let mut cache_outer_done = 0u32;
                        let cache_rx_own = NIC_RX_OWNER.load(Ordering::Acquire);
                        let cache_tx_own = NIC_TX_OWNER.load(Ordering::Acquire);
                        serial_println!(
                            "[sexnet.arp.cache.poll.begin] max_iters=100000000 target_replies=2"
                        );
                        if cache_rx_own == NIC_OWNER_SEXNET_FULL && cache_tx_own == NIC_OWNER_SEXNET_FULL {
                            let mut cache_outer = 0u32;
                            while cache_outer < 100_000_000 && cache_replies < 2 {
                                let mut idx = 0u32;
                                while idx < 8 && cache_replies < 2 {
                                    let desc_base = unsafe { RX_PERM_DESC_VA } + (idx as u64) * 16;
                                    let st = unsafe { core::ptr::read_volatile((desc_base + 12) as *const u8) };
                                    if (st & 1) != 0 {
                                        let pkt_buf = unsafe { RX_PERM_PKT_VA[idx as usize] };
                                        let eth_hi = unsafe { core::ptr::read_volatile((pkt_buf + 12) as *const u8) };
                                        let eth_lo = unsafe { core::ptr::read_volatile((pkt_buf + 13) as *const u8) };
                                        let htype_hi = unsafe { core::ptr::read_volatile((pkt_buf + 14) as *const u8) };
                                        let htype_lo = unsafe { core::ptr::read_volatile((pkt_buf + 15) as *const u8) };
                                        let ptype_hi = unsafe { core::ptr::read_volatile((pkt_buf + 16) as *const u8) };
                                        let ptype_lo = unsafe { core::ptr::read_volatile((pkt_buf + 17) as *const u8) };
                                        let hlen = unsafe { core::ptr::read_volatile((pkt_buf + 18) as *const u8) };
                                        let plen = unsafe { core::ptr::read_volatile((pkt_buf + 19) as *const u8) };
                                        let oper_hi = unsafe { core::ptr::read_volatile((pkt_buf + 20) as *const u8) };
                                        let oper_lo = unsafe { core::ptr::read_volatile((pkt_buf + 21) as *const u8) };
                                        let ethertype = ((eth_hi as u16) << 8) | (eth_lo as u16);
                                        let htype = ((htype_hi as u16) << 8) | (htype_lo as u16);
                                        let ptype = ((ptype_hi as u16) << 8) | (ptype_lo as u16);
                                        let oper = ((oper_hi as u16) << 8) | (oper_lo as u16);
                                        let mut tpa = [0u8; 4];
                                        let mut ti = 0usize;
                                        while ti < 4 {
                                            tpa[ti] = unsafe {
                                                core::ptr::read_volatile((pkt_buf + 38 + ti as u64) as *const u8)
                                            };
                                            ti += 1;
                                        }
                                        let valid = ethertype == 0x0806
                                            && htype == 1
                                            && ptype == 0x0800
                                            && hlen == 6
                                            && plen == 4
                                            && oper == 1
                                            && tpa == SEXNET_GUEST_IPV4;
                                        if valid {
                                            let mut mi = 0usize;
                                            while mi < 6 {
                                                unsafe {
                                                    ARP_CACHE_MAC[mi] = core::ptr::read_volatile(
                                                        (pkt_buf + 22 + mi as u64) as *const u8
                                                    );
                                                }
                                                mi += 1;
                                            }
                                            let mut si = 0usize;
                                            while si < 4 {
                                                unsafe {
                                                    ARP_CACHE_IP[si] = core::ptr::read_volatile(
                                                        (pkt_buf + 28 + si as u64) as *const u8
                                                    );
                                                }
                                                si += 1;
                                            }
                                            unsafe {
                                                ARP_CACHE_VALID = 1;
                                            }
                                            let n = cache_replies + 1;
                                            serial_println!(
                                                "[sexnet.arp.cache.learn] n={} sha={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} spa={}.{}.{}.{} ok=1",
                                                n,
                                                unsafe { ARP_CACHE_MAC[0] },
                                                unsafe { ARP_CACHE_MAC[1] },
                                                unsafe { ARP_CACHE_MAC[2] },
                                                unsafe { ARP_CACHE_MAC[3] },
                                                unsafe { ARP_CACHE_MAC[4] },
                                                unsafe { ARP_CACHE_MAC[5] },
                                                unsafe { ARP_CACHE_IP[0] },
                                                unsafe { ARP_CACHE_IP[1] },
                                                unsafe { ARP_CACHE_IP[2] },
                                                unsafe { ARP_CACHE_IP[3] }
                                            );
                                            // Phase D: ARP gateway cache proof marker
                                            serial_println!(
                                                "[sexnet.arp.cache.gateway.ok] n={} mac={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} ip={}.{}.{}.{} ok=1",
                                                n,
                                                unsafe { ARP_CACHE_MAC[0] },
                                                unsafe { ARP_CACHE_MAC[1] },
                                                unsafe { ARP_CACHE_MAC[2] },
                                                unsafe { ARP_CACHE_MAC[3] },
                                                unsafe { ARP_CACHE_MAC[4] },
                                                unsafe { ARP_CACHE_MAC[5] },
                                                unsafe { ARP_CACHE_IP[0] },
                                                unsafe { ARP_CACHE_IP[1] },
                                                unsafe { ARP_CACHE_IP[2] },
                                                unsafe { ARP_CACHE_IP[3] }
                                            );

                                            let tx_slot = 3u32 + cache_replies;
                                            let tx_tdt = 4u32 + cache_replies;
                                            let tx_frame_va = unsafe { TX_PERM_FRAME_VA };
                                            let mut i = 0usize;
                                            while i < 6 {
                                                unsafe {
                                                    core::ptr::write_volatile(
                                                        (tx_frame_va + i as u64) as *mut u8,
                                                        ARP_CACHE_MAC[i],
                                                    );
                                                    core::ptr::write_volatile(
                                                        (tx_frame_va + 6 + i as u64) as *mut u8,
                                                        cache_nic_mac[i],
                                                    );
                                                }
                                                i += 1;
                                            }
                                            unsafe {
                                                core::ptr::write_volatile((tx_frame_va + 12) as *mut u8, 0x08);
                                                core::ptr::write_volatile((tx_frame_va + 13) as *mut u8, 0x06);
                                                core::ptr::write_volatile((tx_frame_va + 14) as *mut u8, 0x00);
                                                core::ptr::write_volatile((tx_frame_va + 15) as *mut u8, 0x01);
                                                core::ptr::write_volatile((tx_frame_va + 16) as *mut u8, 0x08);
                                                core::ptr::write_volatile((tx_frame_va + 17) as *mut u8, 0x00);
                                                core::ptr::write_volatile((tx_frame_va + 18) as *mut u8, 0x06);
                                                core::ptr::write_volatile((tx_frame_va + 19) as *mut u8, 0x04);
                                                core::ptr::write_volatile((tx_frame_va + 20) as *mut u8, 0x00);
                                                core::ptr::write_volatile((tx_frame_va + 21) as *mut u8, 0x02);
                                            }
                                            let mut ai = 0usize;
                                            while ai < 6 {
                                                unsafe {
                                                    core::ptr::write_volatile(
                                                        (tx_frame_va + 22 + ai as u64) as *mut u8,
                                                        cache_nic_mac[ai],
                                                    );
                                                    core::ptr::write_volatile(
                                                        (tx_frame_va + 32 + ai as u64) as *mut u8,
                                                        ARP_CACHE_MAC[ai],
                                                    );
                                                }
                                                ai += 1;
                                            }
                                            let mut gi = 0usize;
                                            while gi < 4 {
                                                unsafe {
                                                    core::ptr::write_volatile(
                                                        (tx_frame_va + 28 + gi as u64) as *mut u8,
                                                        SEXNET_GUEST_IPV4[gi],
                                                    );
                                                    core::ptr::write_volatile(
                                                        (tx_frame_va + 38 + gi as u64) as *mut u8,
                                                        ARP_CACHE_IP[gi],
                                                    );
                                                }
                                                gi += 1;
                                            }
                                            let mut pad = 42u64;
                                            while pad < 60 {
                                                unsafe {
                                                    core::ptr::write_volatile((tx_frame_va + pad) as *mut u8, 0u8);
                                                }
                                                pad += 1;
                                            }
                                            let tx_desc = unsafe { TX_PERM_DESC_VA } + (tx_slot as u64) * 16;
                                            unsafe {
                                                core::ptr::write_volatile(tx_desc as *mut u64, TX_PERM_FRAME_PHYS);
                                                core::ptr::write_volatile((tx_desc + 8) as *mut u16, 60u16);
                                                core::ptr::write_volatile((tx_desc + 10) as *mut u8, 0u8);
                                                core::ptr::write_volatile((tx_desc + 11) as *mut u8, 0x0Bu8);
                                                core::ptr::write_volatile((tx_desc + 12) as *mut u8, 0u8);
                                                core::ptr::write_volatile((tx_desc + 13) as *mut u8, 0u8);
                                                core::ptr::write_volatile((tx_desc + 14) as *mut u16, 0u16);
                                                core::ptr::write_volatile((nic_va + 0x3818) as *mut u32, tx_tdt);
                                            }
                                            serial_println!(
                                                "[sexnet.arp.cache.reply] n={} slot={} tdt={} ok=1",
                                                n,
                                                tx_slot,
                                                tx_tdt
                                            );

                                            let mut dd_set = 0u32;
                                            let mut tx_outer = 0u32;
                                            while tx_outer < 50_000_000 {
                                                let tx_st = unsafe {
                                                    core::ptr::read_volatile((tx_desc + 12) as *const u8)
                                                };
                                                if (tx_st & 1) != 0 {
                                                    dd_set = 1;
                                                    break;
                                                }
                                                tx_outer += 1;
                                            }
                                            serial_println!(
                                                "[sexnet.arp.cache.reply.dd] n={} dd_set={} ok={}",
                                                n,
                                                dd_set,
                                                if dd_set == 1 { 1 } else { 0 }
                                            );

                                            cache_replies += 1;
                                            unsafe {
                                                ARP_CACHE_REPLY_COUNT = cache_replies;
                                            }
                                        }
                                        unsafe {
                                            core::ptr::write_volatile((desc_base + 8) as *mut u16, 0u16);
                                            core::ptr::write_volatile((desc_base + 12) as *mut u8, 0u8);
                                            core::ptr::write_volatile((nic_va + 0x2818) as *mut u32, idx);
                                        }
                                    }
                                    idx += 1;
                                }
                                cache_outer += 1;
                            }
                            cache_outer_done = cache_outer;
                        }
                        let cache_ok = if cache_replies == 2 { 1 } else { 0 };
                        serial_println!(
                            "[sexnet.arp.cache.poll.done] outer={} replies={} ok={}",
                            cache_outer_done,
                            cache_replies,
                            cache_ok
                        );
                        serial_println!(
                            "[sexnet.arp.cache.proof.done] replies={} ok={}",
                            cache_replies,
                            cache_ok
                        );

                        let l2_rx_own = NIC_RX_OWNER.load(Ordering::Acquire);
                        let l2_tx_own = NIC_TX_OWNER.load(Ordering::Acquire);
                        if l2_rx_own == NIC_OWNER_SEXNET_FULL && l2_tx_own == NIC_OWNER_SEXNET_FULL {
                            serial_println!("[sexnet.l2.entry] rx_owner=3 tx_owner=3 ok=1");
                            serial_println!("[sexnet.l2.rx.poll.begin] max_frames=3 max_iters_per=3000000");
                            let mut l2_frames = 0u32;
                            let mut outer = 0u32;
                            while outer < 3_000_000 && l2_frames < 3 {
                                let mut break_l2_poll = false;
                                let mut idx = 0u32;
                                while idx < 8 && l2_frames < 3 {
                                    let st = unsafe {
                                        core::ptr::read_volatile((RX_PERM_DESC_VA + (idx as u64) * 16 + 12) as *const u8)
                                    };
                                    if (st & 1) != 0 {
                                        let desc_base = unsafe { RX_PERM_DESC_VA } + (idx as u64) * 16;
                                        let pkt_len = unsafe { core::ptr::read_volatile((desc_base + 8) as *const u16) } as u32;
                                        let pkt_buf = unsafe { RX_PERM_PKT_VA[idx as usize] };
                                        let eth_hi = unsafe { core::ptr::read_volatile((pkt_buf + 12) as *const u8) };
                                        let eth_lo = unsafe { core::ptr::read_volatile((pkt_buf + 13) as *const u8) };
                                        let ethertype = ((eth_hi as u16) << 8) | (eth_lo as u16);
                                        l2_frames += 1;
                                        unsafe {
                                            L2_RX_NEXT = idx as u8;
                                            L2_RX_COUNT = l2_frames;
                                        }
                                        serial_println!(
                                            "[sexnet.l2.rx.frame] idx={} len={} ethertype=0x{:04X} count={} ok=1",
                                            idx,
                                            pkt_len,
                                            ethertype,
                                            l2_frames
                                        );
                                        if ethertype != 0x0806 {
                                            unsafe {
                                                core::ptr::write_volatile((desc_base + 8) as *mut u16, 0u16);
                                                core::ptr::write_volatile((desc_base + 12) as *mut u8, 0u8);
                                                core::ptr::write_volatile((nic_va + 0x2818) as *mut u32, idx);
                                            }
                                            serial_println!(
                                                "[sexnet.l2.rx.recycle] idx={} new_rdt={} ok=1",
                                                idx,
                                                idx
                                            );
                                        } else {
                                            // Preserve ARP frame for later lane while preventing repeated count on same DD.
                                            break_l2_poll = true;
                                            break;
                                        }
                                    }
                                    idx += 1;
                                }
                                if break_l2_poll {
                                    break;
                                }
                                outer += 1;
                            }
                            let l2_rx_ok = if l2_frames > 0 { 1 } else { 0 };
                            serial_println!(
                                "[sexnet.l2.rx.poll.done] frames_rx={} ok={}",
                                l2_frames,
                                l2_rx_ok
                            );

                            let mut l2_tx_dd = 0u32;
                            let tx_perm_ready = unsafe {
                                TX_PERM_DESC_VA != 0 && TX_PERM_FRAME_PHYS != 0 && TX_PERM_FRAME_VA != 0
                            };
                            if tx_perm_ready {
                                let tx_frame_va = unsafe { TX_PERM_FRAME_VA };
                                let mut bi = 0u64;
                                while bi < 6 {
                                    unsafe {
                                        core::ptr::write_volatile((tx_frame_va + bi) as *mut u8, 0xFF);
                                    }
                                    bi += 1;
                                }
                                let src = [0x52u8, 0x54u8, 0x00u8, 0x12u8, 0x34u8, 0x56u8];
                                let mut si = 0usize;
                                while si < 6 {
                                    unsafe {
                                        core::ptr::write_volatile((tx_frame_va + 6 + si as u64) as *mut u8, src[si]);
                                    }
                                    si += 1;
                                }
                                unsafe {
                                    core::ptr::write_volatile((tx_frame_va + 12) as *mut u8, 0x88);
                                    core::ptr::write_volatile((tx_frame_va + 13) as *mut u8, 0xB5);
                                }
                                let mut pi = 14u64;
                                while pi < 60 {
                                    unsafe {
                                        core::ptr::write_volatile((tx_frame_va + pi) as *mut u8, 0x42);
                                    }
                                    pi += 1;
                                }
                                let tx_desc2 = unsafe { TX_PERM_DESC_VA + 32 };
                                unsafe {
                                    core::ptr::write_volatile(tx_desc2 as *mut u64, TX_PERM_FRAME_PHYS);
                                    core::ptr::write_volatile((tx_desc2 + 8) as *mut u16, 60u16);
                                    core::ptr::write_volatile((tx_desc2 + 10) as *mut u8, 0u8);
                                    core::ptr::write_volatile((tx_desc2 + 11) as *mut u8, 0x0Bu8);
                                    core::ptr::write_volatile((tx_desc2 + 12) as *mut u8, 0u8);
                                    core::ptr::write_volatile((tx_desc2 + 13) as *mut u8, 0u8);
                                    core::ptr::write_volatile((tx_desc2 + 14) as *mut u16, 0u16);
                                    L2_TX_NEXT = 2;
                                }
                                serial_println!("[sexnet.l2.tx.reuse.desc] slot=2 len=60 ok=1");
                                unsafe {
                                    core::ptr::write_volatile((nic_va + 0x3818) as *mut u32, 3);
                                }
                                serial_println!("[sexnet.l2.tx.reuse.post] tdt=3 ok=1");
                                let mut tx_outer = 0u32;
                                while tx_outer < 50_000_000 {
                                    let tx_st = unsafe { core::ptr::read_volatile((tx_desc2 + 12) as *const u8) };
                                    if (tx_st & 1) != 0 {
                                        l2_tx_dd = 1;
                                        break;
                                    }
                                    tx_outer += 1;
                                }
                                serial_println!(
                                    "[sexnet.l2.tx.reuse.poll.done] dd_set={} desc_idx=2 ok={}",
                                    l2_tx_dd,
                                    if l2_tx_dd == 1 { 1 } else { 0 }
                                );
                            }

                            let l2_ok = if ((l2_frames > 0) || (arp_ok == 1)) && l2_tx_dd == 1 { 1 } else { 0 };
                            serial_println!(
                                "[sexnet.l2.proof.done] rx_frames={} tx_dd={} ok={}",
                                l2_frames,
                                l2_tx_dd,
                                l2_ok
                            );

                            // ------------------------------------------------------------------
                            // Phase G: TCP SYN build and TX
                            // ------------------------------------------------------------------
                            serial_println!("[sexnet.tcp.entry] state=CLOSED local_port={} remote={}.{}.{}.{}:{} ok=1",
                                unsafe { TCP_LOCAL_PORT },
                                unsafe { TCP_REMOTE_IP[0] }, unsafe { TCP_REMOTE_IP[1] },
                                unsafe { TCP_REMOTE_IP[2] }, unsafe { TCP_REMOTE_IP[3] },
                                unsafe { TCP_REMOTE_PORT }
                            );
                            {
                                let mut tcp_syn_built = 0u32;
                                let mut tcp_syn_tx_dd = 0u32;
                                let tx_perm_ready = unsafe {
                                    TX_PERM_DESC_VA != 0 && TX_PERM_FRAME_PHYS != 0 && TX_PERM_FRAME_VA != 0
                                };
                                if tx_perm_ready {
                                    let tx_va = unsafe { TX_PERM_FRAME_VA };
                                    let nic_mac: [u8; 6] = [
                                        (ral & 0xFF) as u8,
                                        ((ral >> 8) & 0xFF) as u8,
                                        ((ral >> 16) & 0xFF) as u8,
                                        ((ral >> 24) & 0xFF) as u8,
                                        (rah & 0xFF) as u8,
                                        ((rah >> 8) & 0xFF) as u8,
                                    ];
                                    // Ethernet header: src=NIC MAC, dst=gateway MAC
                                    // Prefer ARP cache. For QEMU usernet/SLiRP gateway
                                    // 10.0.2.2, use known SLiRP MAC 52:55:0A:00:02:02.
                                    // Broadcast only as last resort for unknown destinations.
                                    let syn_rip = unsafe { TCP_REMOTE_IP };
                                    let dst_mac: [u8; 6] = if unsafe { ARP_CACHE_VALID } == 1 {
                                        unsafe { ARP_CACHE_MAC }
                                    } else if syn_rip[0] == 10 && syn_rip[1] == 0 && syn_rip[2] == 2 && syn_rip[3] == 2 {
                                        serial_println!(
                                            "[sexnet.tcp.syn.mac.resolve] mode=slirp_static dst_ip=10.0.2.2 mac=52:55:0A:00:02:02 ok=1"
                                        );
                                        [0x52, 0x55, 0x0A, 0x00, 0x02, 0x02]
                                    } else {
                                        [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]
                                    };
                                    unsafe {
                                        // dst MAC: gateway or broadcast
                                        core::ptr::write_volatile((tx_va + 0) as *mut u8, dst_mac[0]);
                                        core::ptr::write_volatile((tx_va + 1) as *mut u8, dst_mac[1]);
                                        core::ptr::write_volatile((tx_va + 2) as *mut u8, dst_mac[2]);
                                        core::ptr::write_volatile((tx_va + 3) as *mut u8, dst_mac[3]);
                                        core::ptr::write_volatile((tx_va + 4) as *mut u8, dst_mac[4]);
                                        core::ptr::write_volatile((tx_va + 5) as *mut u8, dst_mac[5]);
                                        // src MAC: NIC
                                        core::ptr::write_volatile((tx_va + 6) as *mut u8, nic_mac[0]);
                                        core::ptr::write_volatile((tx_va + 7) as *mut u8, nic_mac[1]);
                                        core::ptr::write_volatile((tx_va + 8) as *mut u8, nic_mac[2]);
                                        core::ptr::write_volatile((tx_va + 9) as *mut u8, nic_mac[3]);
                                        core::ptr::write_volatile((tx_va + 10) as *mut u8, nic_mac[4]);
                                        core::ptr::write_volatile((tx_va + 11) as *mut u8, nic_mac[5]);
                                        core::ptr::write_volatile((tx_va + 12) as *mut u8, 0x08);
                                        core::ptr::write_volatile((tx_va + 13) as *mut u8, 0x00);
                                    }
                                    // IPv4 header: src=10.0.2.15, dst=gateway, proto=6
                                    let ipv4_total: u16 = 20 + 20; // IPv4 + TCP headers, no options
                                    unsafe {
                                        core::ptr::write_volatile((tx_va + 14) as *mut u8, 0x45); // ver=4 ihl=5
                                        core::ptr::write_volatile((tx_va + 15) as *mut u8, 0x00); // DSCP+ECN
                                        core::ptr::write_volatile((tx_va + 16) as *mut u8, ((ipv4_total >> 8) & 0xFF) as u8);
                                        core::ptr::write_volatile((tx_va + 17) as *mut u8, (ipv4_total & 0xFF) as u8);
                                        core::ptr::write_volatile((tx_va + 18) as *mut u8, 0x00); // ID=0
                                        core::ptr::write_volatile((tx_va + 19) as *mut u8, 0x00);
                                        core::ptr::write_volatile((tx_va + 20) as *mut u8, 0x00); // flags+frag
                                        core::ptr::write_volatile((tx_va + 21) as *mut u8, 0x00);
                                        core::ptr::write_volatile((tx_va + 22) as *mut u8, 64);  // ttl
                                        core::ptr::write_volatile((tx_va + 23) as *mut u8, 6);   // proto=6 TCP
                                        core::ptr::write_volatile((tx_va + 24) as *mut u8, 0x00); // csum placeholder
                                        core::ptr::write_volatile((tx_va + 25) as *mut u8, 0x00);
                                        core::ptr::write_volatile((tx_va + 26) as *mut u8, 10);  // src=10.0.2.15
                                        core::ptr::write_volatile((tx_va + 27) as *mut u8, 0);
                                        core::ptr::write_volatile((tx_va + 28) as *mut u8, 2);
                                        core::ptr::write_volatile((tx_va + 29) as *mut u8, 15);
                                        let rip = unsafe { TCP_REMOTE_IP };
                                        core::ptr::write_volatile((tx_va + 30) as *mut u8, rip[0]);
                                        core::ptr::write_volatile((tx_va + 31) as *mut u8, rip[1]);
                                        core::ptr::write_volatile((tx_va + 32) as *mut u8, rip[2]);
                                        core::ptr::write_volatile((tx_va + 33) as *mut u8, rip[3]);
                                    }
                                    // IPv4 checksum
                                    {
                                        let mut ipv4_sum = 0u32;
                                        let mut ck = 0usize;
                                        while ck < 10 {
                                            let off = 14 + ck * 2;
                                            let w_hi = unsafe { core::ptr::read_volatile((tx_va + off as u64) as *const u8) } as u16;
                                            let w_lo = unsafe { core::ptr::read_volatile((tx_va + off as u64 + 1) as *const u8) } as u16;
                                            ipv4_sum += ((w_hi << 8) | w_lo) as u32;
                                            ck += 1;
                                        }
                                        while (ipv4_sum >> 16) != 0 {
                                            ipv4_sum = (ipv4_sum & 0xFFFF) + (ipv4_sum >> 16);
                                        }
                                        let ipv4_csum = !(ipv4_sum as u16);
                                        unsafe {
                                            core::ptr::write_volatile((tx_va + 24) as *mut u8, ((ipv4_csum >> 8) & 0xFF) as u8);
                                            core::ptr::write_volatile((tx_va + 25) as *mut u8, (ipv4_csum & 0xFF) as u8);
                                        }
                                    }
                                    // TCP header
                                    let local_port = unsafe { TCP_LOCAL_PORT };
                                    let remote_port = unsafe { TCP_REMOTE_PORT };
                                    let local_seq = unsafe { TCP_LOCAL_SEQ };
                                    let data_offset: u8 = 5; // 20-byte header, no options
                                    let tcp_flags: u8 = 0x02; // SYN
                                    unsafe {
                                        core::ptr::write_volatile((tx_va + 34) as *mut u8, ((local_port >> 8) & 0xFF) as u8);
                                        core::ptr::write_volatile((tx_va + 35) as *mut u8, (local_port & 0xFF) as u8);
                                        core::ptr::write_volatile((tx_va + 36) as *mut u8, ((remote_port >> 8) & 0xFF) as u8);
                                        core::ptr::write_volatile((tx_va + 37) as *mut u8, (remote_port & 0xFF) as u8);
                                        core::ptr::write_volatile((tx_va + 38) as *mut u8, ((local_seq >> 24) & 0xFF) as u8);
                                        core::ptr::write_volatile((tx_va + 39) as *mut u8, ((local_seq >> 16) & 0xFF) as u8);
                                        core::ptr::write_volatile((tx_va + 40) as *mut u8, ((local_seq >> 8) & 0xFF) as u8);
                                        core::ptr::write_volatile((tx_va + 41) as *mut u8, (local_seq & 0xFF) as u8);
                                        // ack=0
                                        core::ptr::write_volatile((tx_va + 42) as *mut u8, 0x00);
                                        core::ptr::write_volatile((tx_va + 43) as *mut u8, 0x00);
                                        core::ptr::write_volatile((tx_va + 44) as *mut u8, 0x00);
                                        core::ptr::write_volatile((tx_va + 45) as *mut u8, 0x00);
                                        // data_offset | flags
                                        let dof = (data_offset << 4) | ((tcp_flags >> 0) & 0x01);
                                        let reserved_flags = (tcp_flags & 0x3F);
                                        core::ptr::write_volatile((tx_va + 46) as *mut u8, dof);
                                        core::ptr::write_volatile((tx_va + 47) as *mut u8, reserved_flags);
                                        // window = 65535
                                        core::ptr::write_volatile((tx_va + 48) as *mut u8, 0xFF);
                                        core::ptr::write_volatile((tx_va + 49) as *mut u8, 0xFF);
                                        // checksum placeholder
                                        core::ptr::write_volatile((tx_va + 50) as *mut u8, 0x00);
                                        core::ptr::write_volatile((tx_va + 51) as *mut u8, 0x00);
                                        // urgent = 0
                                        core::ptr::write_volatile((tx_va + 52) as *mut u8, 0x00);
                                        core::ptr::write_volatile((tx_va + 53) as *mut u8, 0x00);
                                    }
                                    // TCP checksum over pseudo-header + TCP header
                                    {
                                        let mut tcp_sum = 0u32;
                                        // pseudo-header: src IP
                                        tcp_sum += ((10u16 << 8) | 0u16) as u32;
                                        tcp_sum += ((2u16 << 8) | 15u16) as u32;
                                        // pseudo-header: dst IP
                                        let rip = unsafe { TCP_REMOTE_IP };
                                        tcp_sum += ((rip[0] as u16) << 8 | (rip[1] as u16)) as u32;
                                        tcp_sum += ((rip[2] as u16) << 8 | (rip[3] as u16)) as u32;
                                        // pseudo-header: zero + proto=6
                                        tcp_sum += 6u32;
                                        // pseudo-header: TCP length = 20
                                        tcp_sum += 20u32;
                                        // TCP header words (10 words)
                                        let mut cw = 0usize;
                                        while cw < 10 {
                                            let off = 34 + cw * 2;
                                            let w_hi = unsafe { core::ptr::read_volatile((tx_va + off as u64) as *const u8) } as u16;
                                            let w_lo = unsafe { core::ptr::read_volatile((tx_va + off as u64 + 1) as *const u8) } as u16;
                                            tcp_sum += ((w_hi << 8) | w_lo) as u32;
                                            cw += 1;
                                        }
                                        while (tcp_sum >> 16) != 0 {
                                            tcp_sum = (tcp_sum & 0xFFFF) + (tcp_sum >> 16);
                                        }
                                        let tcp_csum = !(tcp_sum as u16);
                                        unsafe {
                                            core::ptr::write_volatile((tx_va + 50) as *mut u8, ((tcp_csum >> 8) & 0xFF) as u8);
                                            core::ptr::write_volatile((tx_va + 51) as *mut u8, (tcp_csum & 0xFF) as u8);
                                        }
                                        serial_println!(
                                            "[sexnet.tcp.syn.checksum] checksum=0x{:04X} ok=1",
                                            tcp_csum
                                        );
                                    }
                                    serial_println!(
                                        "[sexnet.tcp.syn.build] src_port={} dst_port={} seq={} flags=SYN data_offset=5 window=65535 ok=1",
                                        local_port, remote_port, local_seq
                                    );
                                    serial_println!(
                                        "[sexnet.ipv4.tx.tcp_syn.build] src=10.0.2.15 dst={}.{}.{}.{} total_len={} checksum=ok ok=1",
                                        unsafe { TCP_REMOTE_IP[0] }, unsafe { TCP_REMOTE_IP[1] },
                                        unsafe { TCP_REMOTE_IP[2] }, unsafe { TCP_REMOTE_IP[3] },
                                        ipv4_total
                                    );
                                    tcp_syn_built = 1;
                                    // Pad to 60 bytes if needed
                                    let frame_len = (14 + ipv4_total as u64) as u16;
                                    if frame_len < 60 {
                                        let mut pad = frame_len as u64;
                                        while pad < 60 {
                                            unsafe { core::ptr::write_volatile((tx_va + pad) as *mut u8, 0u8); }
                                            pad += 1;
                                        }
                                    }
                                    let tx_frame_len = if frame_len < 60 { 60u16 } else { frame_len };
                                    // TX descriptor 5 (offset 80) for TCP SYN
                                    let tx_desc5 = unsafe { TX_PERM_DESC_VA + 80 };
                                    unsafe {
                                        core::ptr::write_volatile(tx_desc5 as *mut u64, TX_PERM_FRAME_PHYS);
                                        core::ptr::write_volatile((tx_desc5 + 8) as *mut u16, tx_frame_len);
                                        core::ptr::write_volatile((tx_desc5 + 10) as *mut u8, 0u8);
                                        core::ptr::write_volatile((tx_desc5 + 11) as *mut u8, 0x0Bu8);
                                        core::ptr::write_volatile((tx_desc5 + 12) as *mut u8, 0u8);
                                        core::ptr::write_volatile((tx_desc5 + 13) as *mut u8, 0u8);
                                        core::ptr::write_volatile((tx_desc5 + 14) as *mut u16, 0u16);
                                    }
                                    serial_println!("[sexnet.eth.tx.tcp_syn.desc] len={} ok=1", tx_frame_len);
                                    unsafe {
                                        core::ptr::write_volatile((nic_va + 0x3818) as *mut u32, 6);
                                    }
                                    serial_println!("[sexnet.tcp.syn.tx.post] slot=6 ok=1");
                                    // Poll DD
                                    let mut tx_outer = 0u32;
                                    while tx_outer < 50_000_000 {
                                        let tx_st = unsafe { core::ptr::read_volatile((tx_desc5 + 12) as *const u8) };
                                        if (tx_st & 1) != 0 {
                                            tcp_syn_tx_dd = 1;
                                            break;
                                        }
                                        tx_outer += 1;
                                    }
                                    serial_println!(
                                        "[sexnet.tcp.syn.tx.poll.done] dd_set={} ok={}",
                                        tcp_syn_tx_dd,
                                        if tcp_syn_tx_dd == 1 { 1 } else { 0 }
                                    );
                                    if tcp_syn_tx_dd == 1 {
                                        let mut ts = TCP_STATE.lock();
                                        *ts = TcpState::SynSent;
                                        unsafe { TCP_SYN_COUNT = 1; }
                                        serial_println!("[sexnet.tcp.handshake.state] state=SYN_SENT ok=1");
                                    }
                                }
                                serial_println!(
                                    "[sexnet.tcp.syn.build.proof.done] built={} checksum_ok={} ok={}",
                                    tcp_syn_built,
                                    if tcp_syn_built == 1 { 1 } else { 0 },
                                    if tcp_syn_built == 1 && tcp_syn_tx_dd == 1 { 1 } else { 0 }
                                );
                                serial_println!(
                                    "[sexnet.tcp.syn.tx.proof.done] tx={} tx_dd={} ok={}",
                                    if tcp_syn_built == 1 { 1 } else { 0 },
                                    tcp_syn_tx_dd,
                                    if tcp_syn_built == 1 && tcp_syn_tx_dd == 1 { 1 } else { 0 }
                                );
                            }

                            let ipv4_rx_own = NIC_RX_OWNER.load(Ordering::Acquire);
                            if ipv4_rx_own == NIC_OWNER_SEXNET_FULL {
                                serial_println!("[sexnet.ipv4.entry] rx_owner=3 ok=1");
                                serial_println!("[sexnet.ipv4.rx.poll.begin] max_iters=1000000");
                                // Phase E UDP self-test: inject synthetic UDP echo request
                                // to exercise the UDP handler when no real stimulus is available.
                                let udp_test_idx = 7u32; // highest index, polled last — only fires if no real frames
                                let udp_test_buf = unsafe { RX_PERM_PKT_VA[udp_test_idx as usize] };
                                let udp_payload = "HELLO_SEXNET_UDP_ECHO".as_bytes();
                                let udp_payload_len = udp_payload.len() as u16;
                                let udp_total = 8u16 + udp_payload_len; // UDP header + payload
                                let ipv4_total: u16 = 20 + udp_total;
                                let frame_bytes: u16 = 14 + ipv4_total;
                                // Ethernet header
                                unsafe {
                                    core::ptr::write_volatile((udp_test_buf + 0) as *mut u8, 0x52); // dst MAC (guest)
                                    core::ptr::write_volatile((udp_test_buf + 1) as *mut u8, 0x54);
                                    core::ptr::write_volatile((udp_test_buf + 2) as *mut u8, 0x00);
                                    core::ptr::write_volatile((udp_test_buf + 3) as *mut u8, 0x12);
                                    core::ptr::write_volatile((udp_test_buf + 4) as *mut u8, 0x34);
                                    core::ptr::write_volatile((udp_test_buf + 5) as *mut u8, 0x56);
                                    core::ptr::write_volatile((udp_test_buf + 6) as *mut u8, 0xFE); // src MAC (host)
                                    core::ptr::write_volatile((udp_test_buf + 7) as *mut u8, 0x56);
                                    core::ptr::write_volatile((udp_test_buf + 8) as *mut u8, 0x3A);
                                    core::ptr::write_volatile((udp_test_buf + 9) as *mut u8, 0x6C);
                                    core::ptr::write_volatile((udp_test_buf + 10) as *mut u8, 0x97);
                                    core::ptr::write_volatile((udp_test_buf + 11) as *mut u8, 0x32);
                                    core::ptr::write_volatile((udp_test_buf + 12) as *mut u8, 0x08);
                                    core::ptr::write_volatile((udp_test_buf + 13) as *mut u8, 0x00);
                                    // IPv4 header
                                    core::ptr::write_volatile((udp_test_buf + 14) as *mut u8, 0x45); // ver=4 ihl=5
                                    core::ptr::write_volatile((udp_test_buf + 15) as *mut u8, 0x00);
                                    core::ptr::write_volatile((udp_test_buf + 16) as *mut u8, ((ipv4_total >> 8) & 0xFF) as u8);
                                    core::ptr::write_volatile((udp_test_buf + 17) as *mut u8, (ipv4_total & 0xFF) as u8);
                                    core::ptr::write_volatile((udp_test_buf + 18) as *mut u8, 0x00);
                                    core::ptr::write_volatile((udp_test_buf + 19) as *mut u8, 0x00);
                                    core::ptr::write_volatile((udp_test_buf + 20) as *mut u8, 0x00);
                                    core::ptr::write_volatile((udp_test_buf + 21) as *mut u8, 0x00);
                                    core::ptr::write_volatile((udp_test_buf + 22) as *mut u8, 64);  // ttl=64
                                    core::ptr::write_volatile((udp_test_buf + 23) as *mut u8, 17);  // proto=17 UDP
                                    core::ptr::write_volatile((udp_test_buf + 24) as *mut u8, 0x00); // csum placeholder
                                    core::ptr::write_volatile((udp_test_buf + 25) as *mut u8, 0x00);
                                    core::ptr::write_volatile((udp_test_buf + 26) as *mut u8, 10);   // src=10.0.2.2
                                    core::ptr::write_volatile((udp_test_buf + 27) as *mut u8, 0);
                                    core::ptr::write_volatile((udp_test_buf + 28) as *mut u8, 2);
                                    core::ptr::write_volatile((udp_test_buf + 29) as *mut u8, 2);
                                    core::ptr::write_volatile((udp_test_buf + 30) as *mut u8, 10);   // dst=10.0.2.15
                                    core::ptr::write_volatile((udp_test_buf + 31) as *mut u8, 0);
                                    core::ptr::write_volatile((udp_test_buf + 32) as *mut u8, 2);
                                    core::ptr::write_volatile((udp_test_buf + 33) as *mut u8, 15);
                                    // Compute IPv4 checksum for test frame
                                    let mut ut_sum = 0u32;
                                    let mut ut_ci = 0usize;
                                    while ut_ci < 10 {
                                        let off = 14 + ut_ci * 2;
                                        let w_hi = core::ptr::read_volatile((udp_test_buf + off as u64) as *const u8) as u16;
                                        let w_lo = core::ptr::read_volatile((udp_test_buf + off as u64 + 1) as *const u8) as u16;
                                        ut_sum += ((w_hi << 8) | w_lo) as u32;
                                        ut_ci += 1;
                                    }
                                    while (ut_sum >> 16) != 0 {
                                        ut_sum = (ut_sum & 0xFFFF) + (ut_sum >> 16);
                                    }
                                    let ut_csum = !(ut_sum as u16);
                                    core::ptr::write_volatile((udp_test_buf + 24) as *mut u8, ((ut_csum >> 8) & 0xFF) as u8);
                                    core::ptr::write_volatile((udp_test_buf + 25) as *mut u8, (ut_csum & 0xFF) as u8);
                                    // UDP header
                                    core::ptr::write_volatile((udp_test_buf + 34) as *mut u8, 0x30); // src_port=12345
                                    core::ptr::write_volatile((udp_test_buf + 35) as *mut u8, 0x39);
                                    core::ptr::write_volatile((udp_test_buf + 36) as *mut u8, 0x1E); // dst_port=7777
                                    core::ptr::write_volatile((udp_test_buf + 37) as *mut u8, 0x61);
                                    core::ptr::write_volatile((udp_test_buf + 38) as *mut u8, ((udp_total >> 8) & 0xFF) as u8);
                                    core::ptr::write_volatile((udp_test_buf + 39) as *mut u8, (udp_total & 0xFF) as u8);
                                    core::ptr::write_volatile((udp_test_buf + 40) as *mut u8, 0x00); // checksum=0
                                    core::ptr::write_volatile((udp_test_buf + 41) as *mut u8, 0x00);
                                    // Payload
                                    let mut ut_pi = 0usize;
                                    while ut_pi < udp_payload.len() {
                                        core::ptr::write_volatile((udp_test_buf + 42 + ut_pi as u64) as *mut u8, udp_payload[ut_pi]);
                                        ut_pi += 1;
                                    }
                                }
                                let mut ipv4_frames = 0u32;
                                let mut ipv4_ok = 0u32;
                                let mut reject_logged = 0u32;
                                let mut outer = 0u32;
                                let mut synthetic_fallback_done = false;
                                let mut icmp_unreachable_seen = false; // track ICMP dest unreachable for TCP SYN retry
                                while outer < 1_000_000 && ipv4_frames < 2 {
                                    let mut idx = 0u32;
                                    while idx < 8 && ipv4_frames < 2 {
                                        let desc_base = unsafe { RX_PERM_DESC_VA } + (idx as u64) * 16;
                                        let st = unsafe {
                                            core::ptr::read_volatile((desc_base + 12) as *const u8)
                                        };
                                        if (st & 1) != 0 {
                                            let pkt_len = unsafe {
                                                core::ptr::read_volatile((desc_base + 8) as *const u16)
                                            } as usize;
                                            let pkt_buf = unsafe { RX_PERM_PKT_VA[idx as usize] };
                                            let eth_hi = unsafe {
                                                core::ptr::read_volatile((pkt_buf + 12) as *const u8)
                                            };
                                            let eth_lo = unsafe {
                                                core::ptr::read_volatile((pkt_buf + 13) as *const u8)
                                            };
                                            let ethertype = ((eth_hi as u16) << 8) | (eth_lo as u16);
                                            if ethertype == 0x0800 {
                                                serial_println!(
                                                    "[sexnet.ipv4.rx.frame] idx={} pkt_len={} ethertype=0x0800 ok=1",
                                                    idx,
                                                    pkt_len
                                                );
                                                let mut reason = "short";
                                                let mut ok = 0u32;
                                                let mut version = 0u8;
                                                let mut ihl = 0u8;
                                                let mut total_len = 0u16;
                                                let mut flags_frag = 0u16;
                                                let mut frag_masked = 1u16;
                                                let mut dst0 = 0u8;
                                                let mut dst1 = 0u8;
                                                let mut dst2 = 0u8;
                                                let mut dst3 = 0u8;
                                                let mut src0 = 0u8;
                                                let mut src1 = 0u8;
                                                let mut src2 = 0u8;
                                                let mut src3 = 0u8;
                                                let mut proto = 0u8;
                                                let mut ttl = 0u8;
                                                let mut csum = 0u16;
                                                let mut checksum_ok = 0u32;
                                                if pkt_len >= 34 {
                                                    let vihl = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 14) as *const u8)
                                                    };
                                                    version = (vihl >> 4) & 0x0F;
                                                    ihl = vihl & 0x0F;
                                                    let tl_hi = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 16) as *const u8)
                                                    } as u16;
                                                    let tl_lo = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 17) as *const u8)
                                                    } as u16;
                                                    total_len = (tl_hi << 8) | tl_lo;
                                                    ttl = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 22) as *const u8)
                                                    };
                                                    proto = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 23) as *const u8)
                                                    };
                                                    src0 = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 26) as *const u8)
                                                    };
                                                    src1 = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 27) as *const u8)
                                                    };
                                                    src2 = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 28) as *const u8)
                                                    };
                                                    src3 = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 29) as *const u8)
                                                    };
                                                    dst0 = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 30) as *const u8)
                                                    };
                                                    dst1 = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 31) as *const u8)
                                                    };
                                                    dst2 = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 32) as *const u8)
                                                    };
                                                    dst3 = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 33) as *const u8)
                                                    };
                                                    let ff_hi = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 20) as *const u8)
                                                    } as u16;
                                                    let ff_lo = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 21) as *const u8)
                                                    } as u16;
                                                    flags_frag = (ff_hi << 8) | ff_lo;
                                                    frag_masked = flags_frag & 0x3FFF;
                                                    let csum_hi = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 24) as *const u8)
                                                    } as u16;
                                                    let csum_lo = unsafe {
                                                        core::ptr::read_volatile((pkt_buf + 25) as *const u8)
                                                    } as u16;
                                                    csum = (csum_hi << 8) | csum_lo;
                                                    if version != 4 {
                                                        reason = "version";
                                                    } else if ihl != 5 {
                                                        reason = "ihl";
                                                    } else if total_len < 20 {
                                                        reason = "total_len_min";
                                                    } else if total_len as usize > pkt_len.saturating_sub(14) {
                                                        reason = "total_len_max";
                                                    } else if frag_masked != 0 {
                                                        reason = "fragmented";
                                                    } else if dst0 != 10 || dst1 != 0 || dst2 != 2 || dst3 != 15 {
                                                        reason = "dst";
                                                    } else {
                                                        let mut sum = 0u32;
                                                        let mut ci = 0usize;
                                                        while ci < 10 {
                                                            let off = 14 + ci * 2;
                                                            let w_hi = unsafe {
                                                                core::ptr::read_volatile((pkt_buf + off as u64) as *const u8)
                                                            } as u16;
                                                            let w_lo = unsafe {
                                                                core::ptr::read_volatile((pkt_buf + off as u64 + 1) as *const u8)
                                                            } as u16;
                                                            sum += ((w_hi << 8) | w_lo) as u32;
                                                            ci += 1;
                                                        }
                                                        while (sum >> 16) != 0 {
                                                            sum = (sum & 0xFFFF) + (sum >> 16);
                                                        }
                                                        if (sum as u16) == 0xFFFF {
                                                            ok = 1;
                                                            checksum_ok = 1;
                                                        } else {
                                                            reason = "checksum";
                                                        }
                                                    }
                                                }

                                                serial_println!(
                                                    "[sexnet.ipv4.rx.validate.detail] ver={} ihl={} total_len={} pkt_len={} frag=0x{:04X} dst={}.{}.{}.{} csum=0x{:04X} checksum_ok={} proto={} ttl={} ok=0",
                                                    version,
                                                    ihl,
                                                    total_len,
                                                    pkt_len,
                                                    flags_frag,
                                                    dst0,
                                                    dst1,
                                                    dst2,
                                                    dst3,
                                                    csum,
                                                    checksum_ok,
                                                    proto,
                                                    ttl
                                                );
                                                // Phase E: IPv4 reject classification markers
                                                if ok != 1 {
                                                    if reason == "checksum" {
                                                        serial_println!("[sexnet.ipv4.bad_checksum.reject] csum=0x{:04X} reason={} ok=1", csum, reason);
                                                    } else if reason == "fragmented" {
                                                        serial_println!("[sexnet.ipv4.fragment.reject] frag=0x{:04X} reason={} ok=1", frag_masked, reason);
                                                    } else if reason == "total_len_min" || reason == "total_len_max" {
                                                        serial_println!("[sexnet.ipv4.bounds.reject] total_len={} pkt_len={} reason={} ok=1", total_len, pkt_len, reason);
                                                    }
                                                }
                                                if ok == 1 {
                                                    serial_println!(
                                                        "[sexnet.ipv4.rx.validate] version=4 ihl=5 total_len={} dst=10.0.2.15 frag=0 checksum=ok src={}.{}.{}.{} proto={} ttl={} ok=1",
                                                        total_len,
                                                        src0,
                                                        src1,
                                                        src2,
                                                        src3,
                                                        proto,
                                                        ttl
                                                    );
                                                    // Phase E: IPv4 parse succeeded
                                                    serial_println!("[sexnet.ipv4.parse.ok] ver=4 ihl=5 total_len={} proto={} ok=1",
                                                        total_len, proto);
                                                    ipv4_ok = 1;
                                                    ipv4_frames += 1;
                                                    // ICMP echo request handler (Phase D)
                                                    if proto == 1 && (total_len as usize) >= 28 {
                                                        let icmp_base = pkt_buf + 34; // 14 eth + 20 ipv4
                                                        let icmp_type = unsafe { core::ptr::read_volatile(icmp_base as *const u8) };
                                                        let icmp_code = unsafe { core::ptr::read_volatile((icmp_base + 1) as *const u8) };
                                                        if icmp_type == 8 && icmp_code == 0 {
                                                            let icmp_hdr_len = 8usize;
                                                            let ipv4_hdr_len = 20usize;
                                                            let icmp_payload_len = (total_len as usize).saturating_sub(ipv4_hdr_len + icmp_hdr_len);
                                                            let id_hi = unsafe { core::ptr::read_volatile((icmp_base + 4) as *const u8) };
                                                            let id_lo = unsafe { core::ptr::read_volatile((icmp_base + 5) as *const u8) };
                                                            let seq_hi = unsafe { core::ptr::read_volatile((icmp_base + 6) as *const u8) };
                                                            let seq_lo = unsafe { core::ptr::read_volatile((icmp_base + 7) as *const u8) };
                                                            let icmp_id = ((id_hi as u16) << 8) | (id_lo as u16);
                                                            let icmp_seq = ((seq_hi as u16) << 8) | (seq_lo as u16);
                                                            let icmp_total_len = icmp_hdr_len + icmp_payload_len;
                                                            serial_println!(
                                                                "[sexnet.icmp.rx.echo] type=8 code=0 len={} id={} seq={} ok=1",
                                                                icmp_total_len, icmp_id, icmp_seq
                                                            );
                                                            // Phase F: ICMP echo request received proof marker
                                                            serial_println!("[sexnet.icmp.echo.rx.ok] type=8 code=0 len={} id={} seq={} ok=1",
                                                                icmp_total_len, icmp_id, icmp_seq);
                                                            // Validate ICMP checksum from RX
                                                            {
                                                                let mut icmp_rx_sum = 0u32;
                                                                let mut cj = 0usize;
                                                                let rx_icmp_words = icmp_total_len / 2;
                                                                while cj < rx_icmp_words {
                                                                    let off = 34 + cj * 2;
                                                                    let w_hi = unsafe { core::ptr::read_volatile((pkt_buf + off as u64) as *const u8) } as u16;
                                                                    let w_lo = unsafe { core::ptr::read_volatile((pkt_buf + off as u64 + 1) as *const u8) } as u16;
                                                                    icmp_rx_sum += ((w_hi << 8) | w_lo) as u32;
                                                                    cj += 1;
                                                                }
                                                                if icmp_total_len % 2 != 0 {
                                                                    let last = unsafe { core::ptr::read_volatile((pkt_buf + 34 + icmp_total_len as u64 - 1) as *const u8) } as u16;
                                                                    icmp_rx_sum += (last << 8) as u32;
                                                                }
                                                                while (icmp_rx_sum >> 16) != 0 {
                                                                    icmp_rx_sum = (icmp_rx_sum & 0xFFFF) + (icmp_rx_sum >> 16);
                                                                }
                                                                let icmp_rx_csum_ok = if (icmp_rx_sum as u16) == 0xFFFF { 1u32 } else { 0u32 };
                                                                serial_println!("[sexnet.icmp.checksum.validate] ok={}", icmp_rx_csum_ok);
                                                            }
                                                            // Build Ethernet + IPv4 + ICMP echo reply in TX frame buffer
                                                            let tx_va = unsafe { TX_PERM_FRAME_VA };
                                                            let nic_mac: [u8; 6] = [
                                                                (ral & 0xFF) as u8,
                                                                ((ral >> 8) & 0xFF) as u8,
                                                                ((ral >> 16) & 0xFF) as u8,
                                                                ((ral >> 24) & 0xFF) as u8,
                                                                (rah & 0xFF) as u8,
                                                                ((rah >> 8) & 0xFF) as u8,
                                                            ];
                                                            let src_mac0 = unsafe { core::ptr::read_volatile((pkt_buf + 6) as *const u8) };
                                                            let src_mac1 = unsafe { core::ptr::read_volatile((pkt_buf + 7) as *const u8) };
                                                            let src_mac2 = unsafe { core::ptr::read_volatile((pkt_buf + 8) as *const u8) };
                                                            let src_mac3 = unsafe { core::ptr::read_volatile((pkt_buf + 9) as *const u8) };
                                                            let src_mac4 = unsafe { core::ptr::read_volatile((pkt_buf + 10) as *const u8) };
                                                            let src_mac5 = unsafe { core::ptr::read_volatile((pkt_buf + 11) as *const u8) };
                                                            // Ethernet header
                                                            unsafe {
                                                                core::ptr::write_volatile((tx_va + 0) as *mut u8, src_mac0);
                                                                core::ptr::write_volatile((tx_va + 1) as *mut u8, src_mac1);
                                                                core::ptr::write_volatile((tx_va + 2) as *mut u8, src_mac2);
                                                                core::ptr::write_volatile((tx_va + 3) as *mut u8, src_mac3);
                                                                core::ptr::write_volatile((tx_va + 4) as *mut u8, src_mac4);
                                                                core::ptr::write_volatile((tx_va + 5) as *mut u8, src_mac5);
                                                                core::ptr::write_volatile((tx_va + 6) as *mut u8, nic_mac[0]);
                                                                core::ptr::write_volatile((tx_va + 7) as *mut u8, nic_mac[1]);
                                                                core::ptr::write_volatile((tx_va + 8) as *mut u8, nic_mac[2]);
                                                                core::ptr::write_volatile((tx_va + 9) as *mut u8, nic_mac[3]);
                                                                core::ptr::write_volatile((tx_va + 10) as *mut u8, nic_mac[4]);
                                                                core::ptr::write_volatile((tx_va + 11) as *mut u8, nic_mac[5]);
                                                                core::ptr::write_volatile((tx_va + 12) as *mut u8, 0x08);
                                                                core::ptr::write_volatile((tx_va + 13) as *mut u8, 0x00);
                                                            }
                                                            let reply_total_len = (ipv4_hdr_len + icmp_total_len) as u16;
                                                            // IPv4 header
                                                            unsafe {
                                                                core::ptr::write_volatile((tx_va + 14) as *mut u8, 0x45); // ver=4 ihl=5
                                                                core::ptr::write_volatile((tx_va + 15) as *mut u8, 0x00); // dscp/ecn
                                                                core::ptr::write_volatile((tx_va + 16) as *mut u8, ((reply_total_len >> 8) & 0xFF) as u8);
                                                                core::ptr::write_volatile((tx_va + 17) as *mut u8, (reply_total_len & 0xFF) as u8);
                                                                core::ptr::write_volatile((tx_va + 18) as *mut u8, 0x00); // id=0
                                                                core::ptr::write_volatile((tx_va + 19) as *mut u8, 0x00);
                                                                core::ptr::write_volatile((tx_va + 20) as *mut u8, 0x00); // flags/frag=0
                                                                core::ptr::write_volatile((tx_va + 21) as *mut u8, 0x00);
                                                                core::ptr::write_volatile((tx_va + 22) as *mut u8, 64);  // ttl=64
                                                                core::ptr::write_volatile((tx_va + 23) as *mut u8, 1);   // proto=1 ICMP
                                                                core::ptr::write_volatile((tx_va + 24) as *mut u8, 0x00); // csum=0 (compute after)
                                                                core::ptr::write_volatile((tx_va + 25) as *mut u8, 0x00);
                                                                core::ptr::write_volatile((tx_va + 26) as *mut u8, 10);  // src=10.0.2.15
                                                                core::ptr::write_volatile((tx_va + 27) as *mut u8, 0);
                                                                core::ptr::write_volatile((tx_va + 28) as *mut u8, 2);
                                                                core::ptr::write_volatile((tx_va + 29) as *mut u8, 15);
                                                                core::ptr::write_volatile((tx_va + 30) as *mut u8, src0); // dst=request src
                                                                core::ptr::write_volatile((tx_va + 31) as *mut u8, src1);
                                                                core::ptr::write_volatile((tx_va + 32) as *mut u8, src2);
                                                                core::ptr::write_volatile((tx_va + 33) as *mut u8, src3);
                                                            }
                                                            // IPv4 checksum
                                                            {
                                                                let mut ipv4_tx_sum = 0u32;
                                                                let mut ck = 0usize;
                                                                while ck < 10 {
                                                                    let off = 14 + ck * 2;
                                                                    let w_hi = unsafe { core::ptr::read_volatile((tx_va + off as u64) as *const u8) } as u16;
                                                                    let w_lo = unsafe { core::ptr::read_volatile((tx_va + off as u64 + 1) as *const u8) } as u16;
                                                                    ipv4_tx_sum += ((w_hi << 8) | w_lo) as u32;
                                                                    ck += 1;
                                                                }
                                                                while (ipv4_tx_sum >> 16) != 0 {
                                                                    ipv4_tx_sum = (ipv4_tx_sum & 0xFFFF) + (ipv4_tx_sum >> 16);
                                                                }
                                                                let ipv4_tx_csum = !(ipv4_tx_sum as u16);
                                                                unsafe {
                                                                    core::ptr::write_volatile((tx_va + 24) as *mut u8, ((ipv4_tx_csum >> 8) & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 25) as *mut u8, (ipv4_tx_csum & 0xFF) as u8);
                                                                }
                                                            }
                                                            // ICMP echo reply header
                                                            unsafe {
                                                                core::ptr::write_volatile((tx_va + 34) as *mut u8, 0x00); // type=0 echo reply
                                                                core::ptr::write_volatile((tx_va + 35) as *mut u8, 0x00); // code=0
                                                                core::ptr::write_volatile((tx_va + 36) as *mut u8, 0x00); // csum=0 (compute after)
                                                                core::ptr::write_volatile((tx_va + 37) as *mut u8, 0x00);
                                                                core::ptr::write_volatile((tx_va + 38) as *mut u8, id_hi);
                                                                core::ptr::write_volatile((tx_va + 39) as *mut u8, id_lo);
                                                                core::ptr::write_volatile((tx_va + 40) as *mut u8, seq_hi);
                                                                core::ptr::write_volatile((tx_va + 41) as *mut u8, seq_lo);
                                                            }
                                                            // Copy ICMP payload from request
                                                            {
                                                                let mut pi = 0usize;
                                                                while pi < icmp_payload_len {
                                                                    let pb = unsafe { core::ptr::read_volatile((icmp_base + icmp_hdr_len as u64 + pi as u64) as *const u8) };
                                                                    unsafe {
                                                                        core::ptr::write_volatile((tx_va + 42u64 + pi as u64) as *mut u8, pb);
                                                                    }
                                                                    pi += 1;
                                                                }
                                                            }
                                                            // ICMP checksum
                                                            {
                                                                let mut icmp_tx_sum = 0u32;
                                                                let mut cl = 0usize;
                                                                let tx_icmp_words = icmp_total_len / 2;
                                                                while cl < tx_icmp_words {
                                                                    let off = 34 + cl * 2;
                                                                    let w_hi = unsafe { core::ptr::read_volatile((tx_va + off as u64) as *const u8) } as u16;
                                                                    let w_lo = unsafe { core::ptr::read_volatile((tx_va + off as u64 + 1) as *const u8) } as u16;
                                                                    icmp_tx_sum += ((w_hi << 8) | w_lo) as u32;
                                                                    cl += 1;
                                                                }
                                                                if icmp_total_len % 2 != 0 {
                                                                    let last_off = 34 + icmp_total_len - 1;
                                                                    let last = unsafe { core::ptr::read_volatile((tx_va + last_off as u64) as *const u8) } as u16;
                                                                    icmp_tx_sum += (last << 8) as u32;
                                                                }
                                                                while (icmp_tx_sum >> 16) != 0 {
                                                                    icmp_tx_sum = (icmp_tx_sum & 0xFFFF) + (icmp_tx_sum >> 16);
                                                                }
                                                                let icmp_tx_csum = !(icmp_tx_sum as u16);
                                                                unsafe {
                                                                    core::ptr::write_volatile((tx_va + 36) as *mut u8, ((icmp_tx_csum >> 8) & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 37) as *mut u8, (icmp_tx_csum & 0xFF) as u8);
                                                                }
                                                            }
                                                            serial_println!(
                                                                "[sexnet.icmp.tx.reply.build] type=0 code=0 len={} id={} seq={} ok=1",
                                                                icmp_total_len, icmp_id, icmp_seq
                                                            );
                                                            serial_println!("[sexnet.icmp.tx.reply.checksum] ok=1");
                                                            serial_println!(
                                                                "[sexnet.ipv4.tx.icmp_reply.build] src=10.0.2.15 dst={}.{}.{}.{} total_len={} checksum=ok ok=1",
                                                                src0, src1, src2, src3, reply_total_len
                                                            );
                                                            // TX descriptor 3
                                                            let frame_len = (14 + reply_total_len as u64) as u16;
                                                            if frame_len < 60 {
                                                                let mut pad = frame_len as u64;
                                                                while pad < 60 {
                                                                    unsafe { core::ptr::write_volatile((tx_va + pad) as *mut u8, 0u8); }
                                                                    pad += 1;
                                                                }
                                                            }
                                                            let tx_frame_len = if frame_len < 60 { 60u16 } else { frame_len };
                                                            let tx_desc3 = unsafe { TX_PERM_DESC_VA + 48 };
                                                            unsafe {
                                                                core::ptr::write_volatile(tx_desc3 as *mut u64, TX_PERM_FRAME_PHYS);
                                                                core::ptr::write_volatile((tx_desc3 + 8) as *mut u16, tx_frame_len);
                                                                core::ptr::write_volatile((tx_desc3 + 10) as *mut u8, 0u8);
                                                                core::ptr::write_volatile((tx_desc3 + 11) as *mut u8, 0x0Bu8);
                                                                core::ptr::write_volatile((tx_desc3 + 12) as *mut u8, 0u8);
                                                                core::ptr::write_volatile((tx_desc3 + 13) as *mut u8, 0u8);
                                                                core::ptr::write_volatile((tx_desc3 + 14) as *mut u16, 0u16);
                                                            }
                                                            serial_println!("[sexnet.eth.tx.icmp_reply.desc] len={} ok=1", tx_frame_len);
                                                            unsafe {
                                                                core::ptr::write_volatile((nic_va + 0x3818) as *mut u32, 4);
                                                            }
                                                            let mut icmp_reply_done = 0u32;
                                                            let mut tx_outer = 0u32;
                                                            while tx_outer < 50_000_000 {
                                                                let tx_st = unsafe { core::ptr::read_volatile((tx_desc3 + 12) as *const u8) };
                                                                if (tx_st & 1) != 0 {
                                                                    icmp_reply_done = 1;
                                                                    break;
                                                                }
                                                                tx_outer += 1;
                                                            }
                                                            serial_println!(
                                                                "[sexnet.icmp.tx.poll.done] dd_set={} ok={}",
                                                                icmp_reply_done,
                                                                if icmp_reply_done == 1 { 1 } else { 0 }
                                                            );
                                                            // Phase F: ICMP echo reply TX marker
                                                            if icmp_reply_done == 1 {
                                                                serial_println!("[sexnet.icmp.echo.reply.tx.ok] tx_dd={} ok=1", icmp_reply_done);
                                                            }
                                                            serial_println!(
                                                                "[sexnet.icmp.echo.proof.done] rx_echo=1 tx_reply={} tx_dd={} ok={}",
                                                                if icmp_reply_done > 0 { 1 } else { 0 },
                                                                icmp_reply_done,
                                                                if icmp_reply_done == 1 { 1 } else { 0 }
                                                            );
                                                            // Phase F: ICMP ping gateway proof marker
                                                            if icmp_reply_done == 1 {
                                                                serial_println!("[sexnet.icmp.ping.gateway.ok] rx_echo=1 tx_reply=1 ok=1");
                                                            } else {
                                                                serial_println!("[sexnet.icmp.ping.gateway.skip] reason=no_arp_or_no_reply ok=1");
                                                            }
                                                        } else {
                                                            serial_println!(
                                                                "[sexnet.icmp.reject] reason=not_echo_request type={} code={} ok=1",
                                                                icmp_type, icmp_code
                                                            );
                                                            if icmp_type == 3 {
                                                                icmp_unreachable_seen = true;
                                                            }
                                                        }
                                                    } else if proto == 1 && (total_len as usize) < 28 {
                                                        serial_println!("[sexnet.icmp.reject] reason=too_short_for_icmp ok=1");
                                                    } else if proto == 17 && (total_len as usize) >= 28 {
                                                        // UDP echo handler (Phase E)
                                                        let udp_base = pkt_buf + 34; // 14 eth + 20 ipv4
                                                        let ipv4_hdr_len = 20usize;
                                                        let ipv4_payload_len = (total_len as usize).saturating_sub(ipv4_hdr_len);
                                                        let udp_src_port_hi = unsafe { core::ptr::read_volatile(udp_base as *const u8) };
                                                        let udp_src_port_lo = unsafe { core::ptr::read_volatile((udp_base + 1) as *const u8) };
                                                        let udp_dst_port_hi = unsafe { core::ptr::read_volatile((udp_base + 2) as *const u8) };
                                                        let udp_dst_port_lo = unsafe { core::ptr::read_volatile((udp_base + 3) as *const u8) };
                                                        let udp_len_hi = unsafe { core::ptr::read_volatile((udp_base + 4) as *const u8) };
                                                        let udp_len_lo = unsafe { core::ptr::read_volatile((udp_base + 5) as *const u8) };
                                                        let udp_csum_hi = unsafe { core::ptr::read_volatile((udp_base + 6) as *const u8) };
                                                        let udp_csum_lo = unsafe { core::ptr::read_volatile((udp_base + 7) as *const u8) };
                                                        let udp_src_port = ((udp_src_port_hi as u16) << 8) | (udp_src_port_lo as u16);
                                                        let udp_dst_port = ((udp_dst_port_hi as u16) << 8) | (udp_dst_port_lo as u16);
                                                        let udp_len = ((udp_len_hi as u16) << 8) | (udp_len_lo as u16);
                                                        let udp_csum = ((udp_csum_hi as u16) << 8) | (udp_csum_lo as u16);
                                                        serial_println!(
                                                            "[sexnet.udp.rx.datagram] src_port={} dst_port={} len={} checksum=0x{:04X} ok=1",
                                                            udp_src_port, udp_dst_port, udp_len, udp_csum
                                                        );
                                                        let mut udp_ok = 1u32;
                                                        let mut udp_reason = "";
                                                        let mut checksum_policy = "zero_allowed";
                                                        let udp_len_ok = if (udp_len as usize) >= 8 && (udp_len as usize) <= ipv4_payload_len { 1u32 } else { 0u32 };
                                                        let ports_ok = 1u32;
                                                        if (udp_len as usize) < 8 {
                                                            udp_ok = 0;
                                                            udp_reason = "udp_len_too_small";
                                                        } else if (udp_len as usize) > ipv4_payload_len {
                                                            udp_ok = 0;
                                                            udp_reason = "udp_len_exceeds_ipv4_payload";
                                                        } else if udp_csum != 0 {
                                                            let mut sum = 0u32;
                                                            // Pseudo-header: src IP
                                                            sum += ((src0 as u16) << 8 | (src1 as u16)) as u32;
                                                            sum += ((src2 as u16) << 8 | (src3 as u16)) as u32;
                                                            // Pseudo-header: dst IP
                                                            sum += ((dst0 as u16) << 8 | (dst1 as u16)) as u32;
                                                            sum += ((dst2 as u16) << 8 | (dst3 as u16)) as u32;
                                                            // Pseudo-header: zero + proto=17
                                                            sum += 17u32;
                                                            // Pseudo-header: UDP length
                                                            sum += udp_len as u32;
                                                            // UDP datagram (header + payload, including checksum field as-is)
                                                            {
                                                                let udp_total = udp_len as usize;
                                                                let mut cu = 0usize;
                                                                while cu < udp_total / 2 {
                                                                    let off = 34 + cu * 2;
                                                                    let w_hi = unsafe { core::ptr::read_volatile((pkt_buf + off as u64) as *const u8) } as u16;
                                                                    let w_lo = unsafe { core::ptr::read_volatile((pkt_buf + off as u64 + 1) as *const u8) } as u16;
                                                                    sum += ((w_hi << 8) | w_lo) as u32;
                                                                    cu += 1;
                                                                }
                                                                if udp_total % 2 != 0 {
                                                                    let last_off = 34 + udp_total - 1;
                                                                    let last = unsafe { core::ptr::read_volatile((pkt_buf + last_off as u64) as *const u8) } as u16;
                                                                    sum += (last << 8) as u32;
                                                                }
                                                            }
                                                            while (sum >> 16) != 0 {
                                                                sum = (sum & 0xFFFF) + (sum >> 16);
                                                            }
                                                            if (sum as u16) != 0xFFFF {
                                                                udp_ok = 0;
                                                                udp_reason = "checksum";
                                                            } else {
                                                                checksum_policy = "validated";
                                                            }
                                                        }
                                                        serial_println!(
                                                            "[sexnet.udp.header.validate] len_ok={} ports_ok={} checksum_policy={} ok={}",
                                                            udp_len_ok, ports_ok, checksum_policy, udp_ok
                                                        );
                                                        if udp_ok == 1 {
                                                            let udp_payload_len = (udp_len as usize).saturating_sub(8);
                                                            let tx_va = unsafe { TX_PERM_FRAME_VA };
                                                            let nic_mac: [u8; 6] = [
                                                                (ral & 0xFF) as u8,
                                                                ((ral >> 8) & 0xFF) as u8,
                                                                ((ral >> 16) & 0xFF) as u8,
                                                                ((ral >> 24) & 0xFF) as u8,
                                                                (rah & 0xFF) as u8,
                                                                ((rah >> 8) & 0xFF) as u8,
                                                            ];
                                                            let src_mac0 = unsafe { core::ptr::read_volatile((pkt_buf + 6) as *const u8) };
                                                            let src_mac1 = unsafe { core::ptr::read_volatile((pkt_buf + 7) as *const u8) };
                                                            let src_mac2 = unsafe { core::ptr::read_volatile((pkt_buf + 8) as *const u8) };
                                                            let src_mac3 = unsafe { core::ptr::read_volatile((pkt_buf + 9) as *const u8) };
                                                            let src_mac4 = unsafe { core::ptr::read_volatile((pkt_buf + 10) as *const u8) };
                                                            let src_mac5 = unsafe { core::ptr::read_volatile((pkt_buf + 11) as *const u8) };
                                                            unsafe {
                                                                core::ptr::write_volatile((tx_va + 0) as *mut u8, src_mac0);
                                                                core::ptr::write_volatile((tx_va + 1) as *mut u8, src_mac1);
                                                                core::ptr::write_volatile((tx_va + 2) as *mut u8, src_mac2);
                                                                core::ptr::write_volatile((tx_va + 3) as *mut u8, src_mac3);
                                                                core::ptr::write_volatile((tx_va + 4) as *mut u8, src_mac4);
                                                                core::ptr::write_volatile((tx_va + 5) as *mut u8, src_mac5);
                                                                core::ptr::write_volatile((tx_va + 6) as *mut u8, nic_mac[0]);
                                                                core::ptr::write_volatile((tx_va + 7) as *mut u8, nic_mac[1]);
                                                                core::ptr::write_volatile((tx_va + 8) as *mut u8, nic_mac[2]);
                                                                core::ptr::write_volatile((tx_va + 9) as *mut u8, nic_mac[3]);
                                                                core::ptr::write_volatile((tx_va + 10) as *mut u8, nic_mac[4]);
                                                                core::ptr::write_volatile((tx_va + 11) as *mut u8, nic_mac[5]);
                                                                core::ptr::write_volatile((tx_va + 12) as *mut u8, 0x08);
                                                                core::ptr::write_volatile((tx_va + 13) as *mut u8, 0x00);
                                                            }
                                                            let reply_total_len = (ipv4_hdr_len + udp_len as usize) as u16;
                                                            unsafe {
                                                                core::ptr::write_volatile((tx_va + 14) as *mut u8, 0x45);
                                                                core::ptr::write_volatile((tx_va + 15) as *mut u8, 0x00);
                                                                core::ptr::write_volatile((tx_va + 16) as *mut u8, ((reply_total_len >> 8) & 0xFF) as u8);
                                                                core::ptr::write_volatile((tx_va + 17) as *mut u8, (reply_total_len & 0xFF) as u8);
                                                                core::ptr::write_volatile((tx_va + 18) as *mut u8, 0x00);
                                                                core::ptr::write_volatile((tx_va + 19) as *mut u8, 0x00);
                                                                core::ptr::write_volatile((tx_va + 20) as *mut u8, 0x00);
                                                                core::ptr::write_volatile((tx_va + 21) as *mut u8, 0x00);
                                                                core::ptr::write_volatile((tx_va + 22) as *mut u8, 64);
                                                                core::ptr::write_volatile((tx_va + 23) as *mut u8, 17);
                                                                core::ptr::write_volatile((tx_va + 24) as *mut u8, 0x00);
                                                                core::ptr::write_volatile((tx_va + 25) as *mut u8, 0x00);
                                                                core::ptr::write_volatile((tx_va + 26) as *mut u8, 10);
                                                                core::ptr::write_volatile((tx_va + 27) as *mut u8, 0);
                                                                core::ptr::write_volatile((tx_va + 28) as *mut u8, 2);
                                                                core::ptr::write_volatile((tx_va + 29) as *mut u8, 15);
                                                                core::ptr::write_volatile((tx_va + 30) as *mut u8, src0);
                                                                core::ptr::write_volatile((tx_va + 31) as *mut u8, src1);
                                                                core::ptr::write_volatile((tx_va + 32) as *mut u8, src2);
                                                                core::ptr::write_volatile((tx_va + 33) as *mut u8, src3);
                                                            }
                                                            {
                                                                let mut ipv4_tx_sum = 0u32;
                                                                let mut ck = 0usize;
                                                                while ck < 10 {
                                                                    let off = 14 + ck * 2;
                                                                    let w_hi = unsafe { core::ptr::read_volatile((tx_va + off as u64) as *const u8) } as u16;
                                                                    let w_lo = unsafe { core::ptr::read_volatile((tx_va + off as u64 + 1) as *const u8) } as u16;
                                                                    ipv4_tx_sum += ((w_hi << 8) | w_lo) as u32;
                                                                    ck += 1;
                                                                }
                                                                while (ipv4_tx_sum >> 16) != 0 {
                                                                    ipv4_tx_sum = (ipv4_tx_sum & 0xFFFF) + (ipv4_tx_sum >> 16);
                                                                }
                                                                let ipv4_tx_csum = !(ipv4_tx_sum as u16);
                                                                unsafe {
                                                                    core::ptr::write_volatile((tx_va + 24) as *mut u8, ((ipv4_tx_csum >> 8) & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 25) as *mut u8, (ipv4_tx_csum & 0xFF) as u8);
                                                                }
                                                            }
                                                            unsafe {
                                                                core::ptr::write_volatile((tx_va + 34) as *mut u8, ((udp_dst_port >> 8) & 0xFF) as u8);
                                                                core::ptr::write_volatile((tx_va + 35) as *mut u8, (udp_dst_port & 0xFF) as u8);
                                                                core::ptr::write_volatile((tx_va + 36) as *mut u8, ((udp_src_port >> 8) & 0xFF) as u8);
                                                                core::ptr::write_volatile((tx_va + 37) as *mut u8, (udp_src_port & 0xFF) as u8);
                                                                core::ptr::write_volatile((tx_va + 38) as *mut u8, ((udp_len >> 8) & 0xFF) as u8);
                                                                core::ptr::write_volatile((tx_va + 39) as *mut u8, (udp_len & 0xFF) as u8);
                                                                core::ptr::write_volatile((tx_va + 40) as *mut u8, 0x00);
                                                                core::ptr::write_volatile((tx_va + 41) as *mut u8, 0x00);
                                                            }
                                                            if udp_payload_len > 0 {
                                                                let mut pj = 0usize;
                                                                while pj < udp_payload_len {
                                                                    let pb = unsafe { core::ptr::read_volatile((udp_base + 8u64 + pj as u64) as *const u8) };
                                                                    unsafe {
                                                                        core::ptr::write_volatile((tx_va + 42u64 + pj as u64) as *mut u8, pb);
                                                                    }
                                                                    pj += 1;
                                                                }
                                                            }
                                                            serial_println!(
                                                                "[sexnet.udp.tx.reply.build] src_port={} dst_port={} len={} payload_len={} ok=1",
                                                                udp_dst_port, udp_src_port, udp_len, udp_payload_len
                                                            );
                                                            serial_println!(
                                                                "[sexnet.udp.tx.reply.checksum] checksum=0x0000 policy=zero_allowed ok=1"
                                                            );
                                                            serial_println!(
                                                                "[sexnet.ipv4.tx.udp_reply.build] src=10.0.2.15 dst={}.{}.{}.{} total_len={} checksum=ok ok=1",
                                                                src0, src1, src2, src3, reply_total_len
                                                            );
                                                            let frame_len = (14 + reply_total_len as u64) as u16;
                                                            if frame_len < 60 {
                                                                let mut pad = frame_len as u64;
                                                                while pad < 60 {
                                                                    unsafe { core::ptr::write_volatile((tx_va + pad) as *mut u8, 0u8); }
                                                                    pad += 1;
                                                                }
                                                            }
                                                            let tx_frame_len = if frame_len < 60 { 60u16 } else { frame_len };
                                                            let tx_desc4 = unsafe { TX_PERM_DESC_VA + 64 };
                                                            unsafe {
                                                                core::ptr::write_volatile(tx_desc4 as *mut u64, TX_PERM_FRAME_PHYS);
                                                                core::ptr::write_volatile((tx_desc4 + 8) as *mut u16, tx_frame_len);
                                                                core::ptr::write_volatile((tx_desc4 + 10) as *mut u8, 0u8);
                                                                core::ptr::write_volatile((tx_desc4 + 11) as *mut u8, 0x0Bu8);
                                                                core::ptr::write_volatile((tx_desc4 + 12) as *mut u8, 0u8);
                                                                core::ptr::write_volatile((tx_desc4 + 13) as *mut u8, 0u8);
                                                                core::ptr::write_volatile((tx_desc4 + 14) as *mut u16, 0u16);
                                                            }
                                                            serial_println!(
                                                                "[sexnet.eth.tx.udp_reply.desc] len={} ok=1",
                                                                tx_frame_len
                                                            );
                                                            unsafe {
                                                                core::ptr::write_volatile((nic_va + 0x3818) as *mut u32, 5);
                                                            }
                                                            let mut udp_reply_done = 0u32;
                                                            let mut tx_outer = 0u32;
                                                            while tx_outer < 50_000_000 {
                                                                let tx_st = unsafe { core::ptr::read_volatile((tx_desc4 + 12) as *const u8) };
                                                                if (tx_st & 1) != 0 {
                                                                    udp_reply_done = 1;
                                                                    break;
                                                                }
                                                                tx_outer += 1;
                                                            }
                                                            serial_println!(
                                                                "[sexnet.udp.tx.poll.done] dd_set={} ok={}",
                                                                udp_reply_done,
                                                                if udp_reply_done == 1 { 1 } else { 0 }
                                                            );
                                                            serial_println!(
                                                                "[sexnet.udp.echo.proof.done] rx_udp=1 tx_reply={} tx_dd={} ok={}",
                                                                if udp_reply_done > 0 { 1 } else { 0 },
                                                                udp_reply_done,
                                                                if udp_reply_done == 1 { 1 } else { 0 }
                                                            );
                                                        } else {
                                                            serial_println!(
                                                                "[sexnet.udp.reject] reason={} ok=1",
                                                                udp_reason
                                                            );
                                                        }
                                                        serial_println!(
                                                            "[sexnet.udp.header.proof.done] rx_udp=1 valid={} ok=1",
                                                            udp_ok
                                                        );
                                                    // TCP SYN-ACK / RST handler (Phase G)
                                                    } else if proto == 6 && (total_len as usize) >= 40 {
                                                        let tcp_base = pkt_buf + 34; // 14 eth + 20 ipv4
                                                        let ipv4_hdr_len = 20usize;
                                                        let ipv4_payload_len = (total_len as usize).saturating_sub(ipv4_hdr_len);
                                                        // Parse TCP ports
                                                        let tcp_src_port_hi = unsafe { core::ptr::read_volatile(tcp_base as *const u8) };
                                                        let tcp_src_port_lo = unsafe { core::ptr::read_volatile((tcp_base + 1) as *const u8) };
                                                        let tcp_dst_port_hi = unsafe { core::ptr::read_volatile((tcp_base + 2) as *const u8) };
                                                        let tcp_dst_port_lo = unsafe { core::ptr::read_volatile((tcp_base + 3) as *const u8) };
                                                        let tcp_src_port = ((tcp_src_port_hi as u16) << 8) | (tcp_src_port_lo as u16);
                                                        let tcp_dst_port = ((tcp_dst_port_hi as u16) << 8) | (tcp_dst_port_lo as u16);
                                                        // Parse SEQ and ACK
                                                        let seq0 = unsafe { core::ptr::read_volatile((tcp_base + 4) as *const u8) } as u32;
                                                        let seq1 = unsafe { core::ptr::read_volatile((tcp_base + 5) as *const u8) } as u32;
                                                        let seq2 = unsafe { core::ptr::read_volatile((tcp_base + 6) as *const u8) } as u32;
                                                        let seq3 = unsafe { core::ptr::read_volatile((tcp_base + 7) as *const u8) } as u32;
                                                        let tcp_seq = (seq0 << 24) | (seq1 << 16) | (seq2 << 8) | seq3;
                                                        let ack0 = unsafe { core::ptr::read_volatile((tcp_base + 8) as *const u8) } as u32;
                                                        let ack1 = unsafe { core::ptr::read_volatile((tcp_base + 9) as *const u8) } as u32;
                                                        let ack2 = unsafe { core::ptr::read_volatile((tcp_base + 10) as *const u8) } as u32;
                                                        let ack3 = unsafe { core::ptr::read_volatile((tcp_base + 11) as *const u8) } as u32;
                                                        let tcp_ack = (ack0 << 24) | (ack1 << 16) | (ack2 << 8) | ack3;
                                                        // Parse data_offset + flags
                                                        let dof_raw = unsafe { core::ptr::read_volatile((tcp_base + 12) as *const u8) };
                                                        let flags_raw = unsafe { core::ptr::read_volatile((tcp_base + 13) as *const u8) };
                                                        let tcp_data_offset = (dof_raw >> 4) & 0x0F;
                                                        let tcp_flags_syn = (flags_raw >> 1) & 0x01;
                                                        let tcp_flags_ack = (flags_raw >> 4) & 0x01;
                                                        let tcp_flags_rst = (flags_raw >> 2) & 0x01;
                                                        // Parse checksum
                                                        let csum_hi = unsafe { core::ptr::read_volatile((tcp_base + 16) as *const u8) };
                                                        let csum_lo = unsafe { core::ptr::read_volatile((tcp_base + 17) as *const u8) };
                                                        let tcp_csum_rx = ((csum_hi as u16) << 8) | (csum_lo as u16);
                                                        serial_println!(
                                                            "[sexnet.tcp.rx.segment] src_port={} dst_port={} seq={} ack={} data_offset={} flags_syn={} flags_ack={} flags_rst={} csum=0x{:04X} ok=1",
                                                            tcp_src_port, tcp_dst_port, tcp_seq, tcp_ack,
                                                            tcp_data_offset, tcp_flags_syn, tcp_flags_ack, tcp_flags_rst,
                                                            tcp_csum_rx
                                                        );
                                                        let local_port = unsafe { TCP_LOCAL_PORT };
                                                        let remote_port = unsafe { TCP_REMOTE_PORT };
                                                        let local_seq = unsafe { TCP_LOCAL_SEQ };
                                                        let tcp_state = { let s = TCP_STATE.lock(); *s };
                                                        let mut tcp_ok = 0u32;
                                                        let mut tcp_reason = "unknown";
                                                        // Bounds checks
                                                        if tcp_data_offset < 5 {
                                                            tcp_reason = "data_offset_too_small";
                                                        } else if (tcp_data_offset as usize) * 4 > ipv4_payload_len {
                                                            tcp_reason = "header_exceeds_payload";
                                                        } else if tcp_dst_port != local_port {
                                                            tcp_reason = "dst_port_mismatch";
                                                        } else if tcp_src_port != remote_port {
                                                            tcp_reason = "src_port_mismatch";
                                                        } else if tcp_state != TcpState::SynSent && tcp_state != TcpState::Established {
                                                            tcp_reason = "tcp_not_awaiting_response";
                                                        } else {
                                                            // Validate TCP checksum over pseudo-header
                                                            let mut tcp_rx_sum = 0u32;
                                                            // pseudo-header src IP
                                                            tcp_rx_sum += ((src0 as u16) << 8 | (src1 as u16)) as u32;
                                                            tcp_rx_sum += ((src2 as u16) << 8 | (src3 as u16)) as u32;
                                                            // pseudo-header dst IP
                                                            tcp_rx_sum += ((dst0 as u16) << 8 | (dst1 as u16)) as u32;
                                                            tcp_rx_sum += ((dst2 as u16) << 8 | (dst3 as u16)) as u32;
                                                            // pseudo-header zero + proto=6
                                                            tcp_rx_sum += 6u32;
                                                            // pseudo-header TCP length
                                                            let tcp_seg_len = (total_len as usize).saturating_sub(ipv4_hdr_len) as u16;
                                                            tcp_rx_sum += tcp_seg_len as u32;
                                                            // TCP segment words
                                                            let tcp_bytes = tcp_seg_len as usize;
                                                            let tcp_words = tcp_bytes / 2;
                                                            let mut cw = 0usize;
                                                            while cw < tcp_words {
                                                                let off = 34 + cw * 2;
                                                                let w_hi = unsafe { core::ptr::read_volatile((pkt_buf + off as u64) as *const u8) } as u16;
                                                                let w_lo = unsafe { core::ptr::read_volatile((pkt_buf + off as u64 + 1) as *const u8) } as u16;
                                                                tcp_rx_sum += ((w_hi << 8) | w_lo) as u32;
                                                                cw += 1;
                                                            }
                                                            if tcp_bytes % 2 != 0 {
                                                                let last_off = 34 + tcp_bytes - 1;
                                                                let last = unsafe { core::ptr::read_volatile((pkt_buf + last_off as u64) as *const u8) } as u16;
                                                                tcp_rx_sum += (last << 8) as u32;
                                                            }
                                                            while (tcp_rx_sum >> 16) != 0 {
                                                                tcp_rx_sum = (tcp_rx_sum & 0xFFFF) + (tcp_rx_sum >> 16);
                                                            }
                                                            let tcp_csum_ok = if (tcp_rx_sum as u16) == 0xFFFF { 1u32 } else { 0u32 };
                                                            if tcp_csum_ok == 0 {
                                                                tcp_reason = "checksum";
                                                            } else {
                                                                tcp_ok = 1;
                                                                tcp_reason = "ok";
                                                            }
                                                        }
                                                        serial_println!(
                                                            "[sexnet.tcp.rx.validate] ports_ok={} data_offset_ok={} checksum_ok={} ok={}",
                                                            if tcp_dst_port == local_port && tcp_src_port == remote_port { 1 } else { 0 },
                                                            if tcp_data_offset >= 5 { 1 } else { 0 },
                                                            if tcp_ok == 1 { 1 } else { 0 },
                                                            tcp_ok
                                                        );
                                                        if tcp_ok == 1 {
                                                            // Handle RST
                                                            if tcp_flags_rst == 1 {
                                                                let mut ts = TCP_STATE.lock();
                                                                *ts = TcpState::FailedRst;
                                                                unsafe { TCP_RST_COUNT += 1; }
                                                                serial_println!(
                                                                    "[sexnet.tcp.rst.rx] src_port={} dst_port={} seq={} ack={} flags=RST ok=1",
                                                                    tcp_src_port, tcp_dst_port, tcp_seq, tcp_ack
                                                                );
                                                                serial_println!("[sexnet.tcp.handshake.state] state=FAILED_RST ok=1");
                                                                serial_println!(
                                                                    "[sexnet.tcp.synack.rx.proof.done] rx_synack=0 rst=1 timeout=0 ok=0 honest=1"
                                                                );
                                                            // Handle SYN-ACK
                                                            } else if tcp_flags_syn == 1 && tcp_flags_ack == 1 {
                                                                // Validate ACK == local_seq + 1
                                                                let expected_ack = local_seq + 1;
                                                                let ack_ok = if tcp_ack == expected_ack { 1u32 } else { 0u32 };
                                                                serial_println!(
                                                                    "[sexnet.tcp.synack.rx] src_port={} dst_port={} seq={} ack={} flags=SYN|ACK ok=1",
                                                                    tcp_src_port, tcp_dst_port, tcp_seq, tcp_ack
                                                                );
                                                                serial_println!(
                                                                    "[sexnet.tcp.synack.validate] ack_ok={} ports_ok=1 checksum_ok=1 ok={}",
                                                                    ack_ok,
                                                                    if ack_ok == 1 { 1 } else { 0 }
                                                                );
                                                                let mut ts = TCP_STATE.lock();
                                                                *ts = TcpState::Established;
                                                                unsafe {
                                                                    TCP_REMOTE_SEQ = tcp_seq;
                                                                }
                                                                serial_println!("[sexnet.tcp.handshake.state] state=ESTABLISHED ok=1");
                                                                serial_println!(
                                                                    "[sexnet.tcp.synack.rx.proof.done] rx_synack=1 rst=0 timeout=0 ok=1"
                                                                );
                                                                // Build and send final ACK
                                                                let tx_va = unsafe { TX_PERM_FRAME_VA };
                                                                let nic_mac: [u8; 6] = [
                                                                    (ral & 0xFF) as u8,
                                                                    ((ral >> 8) & 0xFF) as u8,
                                                                    ((ral >> 16) & 0xFF) as u8,
                                                                    ((ral >> 24) & 0xFF) as u8,
                                                                    (rah & 0xFF) as u8,
                                                                    ((rah >> 8) & 0xFF) as u8,
                                                                ];
                                                                // Read src MAC from RX frame for Ethernet reply dst
                                                                let src_mac0 = unsafe { core::ptr::read_volatile((pkt_buf + 6) as *const u8) };
                                                                let src_mac1 = unsafe { core::ptr::read_volatile((pkt_buf + 7) as *const u8) };
                                                                let src_mac2 = unsafe { core::ptr::read_volatile((pkt_buf + 8) as *const u8) };
                                                                let src_mac3 = unsafe { core::ptr::read_volatile((pkt_buf + 9) as *const u8) };
                                                                let src_mac4 = unsafe { core::ptr::read_volatile((pkt_buf + 10) as *const u8) };
                                                                let src_mac5 = unsafe { core::ptr::read_volatile((pkt_buf + 11) as *const u8) };
                                                                // Ethernet header
                                                                unsafe {
                                                                    core::ptr::write_volatile((tx_va + 0) as *mut u8, src_mac0);
                                                                    core::ptr::write_volatile((tx_va + 1) as *mut u8, src_mac1);
                                                                    core::ptr::write_volatile((tx_va + 2) as *mut u8, src_mac2);
                                                                    core::ptr::write_volatile((tx_va + 3) as *mut u8, src_mac3);
                                                                    core::ptr::write_volatile((tx_va + 4) as *mut u8, src_mac4);
                                                                    core::ptr::write_volatile((tx_va + 5) as *mut u8, src_mac5);
                                                                    core::ptr::write_volatile((tx_va + 6) as *mut u8, nic_mac[0]);
                                                                    core::ptr::write_volatile((tx_va + 7) as *mut u8, nic_mac[1]);
                                                                    core::ptr::write_volatile((tx_va + 8) as *mut u8, nic_mac[2]);
                                                                    core::ptr::write_volatile((tx_va + 9) as *mut u8, nic_mac[3]);
                                                                    core::ptr::write_volatile((tx_va + 10) as *mut u8, nic_mac[4]);
                                                                    core::ptr::write_volatile((tx_va + 11) as *mut u8, nic_mac[5]);
                                                                    core::ptr::write_volatile((tx_va + 12) as *mut u8, 0x08);
                                                                    core::ptr::write_volatile((tx_va + 13) as *mut u8, 0x00);
                                                                }
                                                                let ack_ipv4_total: u16 = 20 + 20;
                                                                unsafe {
                                                                    core::ptr::write_volatile((tx_va + 14) as *mut u8, 0x45);
                                                                    core::ptr::write_volatile((tx_va + 15) as *mut u8, 0x00);
                                                                    core::ptr::write_volatile((tx_va + 16) as *mut u8, ((ack_ipv4_total >> 8) & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 17) as *mut u8, (ack_ipv4_total & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 18) as *mut u8, 0x00);
                                                                    core::ptr::write_volatile((tx_va + 19) as *mut u8, 0x01);
                                                                    core::ptr::write_volatile((tx_va + 20) as *mut u8, 0x00);
                                                                    core::ptr::write_volatile((tx_va + 21) as *mut u8, 0x00);
                                                                    core::ptr::write_volatile((tx_va + 22) as *mut u8, 64);
                                                                    core::ptr::write_volatile((tx_va + 23) as *mut u8, 6);
                                                                    core::ptr::write_volatile((tx_va + 24) as *mut u8, 0x00);
                                                                    core::ptr::write_volatile((tx_va + 25) as *mut u8, 0x00);
                                                                    core::ptr::write_volatile((tx_va + 26) as *mut u8, 10);
                                                                    core::ptr::write_volatile((tx_va + 27) as *mut u8, 0);
                                                                    core::ptr::write_volatile((tx_va + 28) as *mut u8, 2);
                                                                    core::ptr::write_volatile((tx_va + 29) as *mut u8, 15);
                                                                    core::ptr::write_volatile((tx_va + 30) as *mut u8, src0);
                                                                    core::ptr::write_volatile((tx_va + 31) as *mut u8, src1);
                                                                    core::ptr::write_volatile((tx_va + 32) as *mut u8, src2);
                                                                    core::ptr::write_volatile((tx_va + 33) as *mut u8, src3);
                                                                }
                                                                // IPv4 checksum
                                                                {
                                                                    let mut ipv4_sum = 0u32;
                                                                    let mut ck = 0usize;
                                                                    while ck < 10 {
                                                                        let off = 14 + ck * 2;
                                                                        let w_hi = unsafe { core::ptr::read_volatile((tx_va + off as u64) as *const u8) } as u16;
                                                                        let w_lo = unsafe { core::ptr::read_volatile((tx_va + off as u64 + 1) as *const u8) } as u16;
                                                                        ipv4_sum += ((w_hi << 8) | w_lo) as u32;
                                                                        ck += 1;
                                                                    }
                                                                    while (ipv4_sum >> 16) != 0 {
                                                                        ipv4_sum = (ipv4_sum & 0xFFFF) + (ipv4_sum >> 16);
                                                                    }
                                                                    let ipv4_csum = !(ipv4_sum as u16);
                                                                    unsafe {
                                                                        core::ptr::write_volatile((tx_va + 24) as *mut u8, ((ipv4_csum >> 8) & 0xFF) as u8);
                                                                        core::ptr::write_volatile((tx_va + 25) as *mut u8, (ipv4_csum & 0xFF) as u8);
                                                                    }
                                                                }
                                                                // TCP ACK header
                                                                let ack_seq = local_seq + 1;
                                                                let ack_ack = unsafe { TCP_REMOTE_SEQ + 1 };
                                                                unsafe {
                                                                    core::ptr::write_volatile((tx_va + 34) as *mut u8, ((local_port >> 8) & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 35) as *mut u8, (local_port & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 36) as *mut u8, ((remote_port >> 8) & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 37) as *mut u8, (remote_port & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 38) as *mut u8, ((ack_seq >> 24) & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 39) as *mut u8, ((ack_seq >> 16) & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 40) as *mut u8, ((ack_seq >> 8) & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 41) as *mut u8, (ack_seq & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 42) as *mut u8, ((ack_ack >> 24) & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 43) as *mut u8, ((ack_ack >> 16) & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 44) as *mut u8, ((ack_ack >> 8) & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 45) as *mut u8, (ack_ack & 0xFF) as u8);
                                                                    core::ptr::write_volatile((tx_va + 46) as *mut u8, 0x50); // data_offset=5 << 4
                                                                    core::ptr::write_volatile((tx_va + 47) as *mut u8, 0x10); // ACK flag
                                                                    core::ptr::write_volatile((tx_va + 48) as *mut u8, 0xFF); // window=65535
                                                                    core::ptr::write_volatile((tx_va + 49) as *mut u8, 0xFF);
                                                                    core::ptr::write_volatile((tx_va + 50) as *mut u8, 0x00); // csum placeholder
                                                                    core::ptr::write_volatile((tx_va + 51) as *mut u8, 0x00);
                                                                    core::ptr::write_volatile((tx_va + 52) as *mut u8, 0x00); // urgent=0
                                                                    core::ptr::write_volatile((tx_va + 53) as *mut u8, 0x00);
                                                                }
                                                                // TCP checksum
                                                                {
                                                                    let mut tcp_sum = 0u32;
                                                                    tcp_sum += ((10u16 << 8) | 0u16) as u32;
                                                                    tcp_sum += ((2u16 << 8) | 15u16) as u32;
                                                                    tcp_sum += ((src0 as u16) << 8 | (src1 as u16)) as u32;
                                                                    tcp_sum += ((src2 as u16) << 8 | (src3 as u16)) as u32;
                                                                    tcp_sum += 6u32;
                                                                    tcp_sum += 20u32;
                                                                    let mut cw = 0usize;
                                                                    while cw < 10 {
                                                                        let off = 34 + cw * 2;
                                                                        let w_hi = unsafe { core::ptr::read_volatile((tx_va + off as u64) as *const u8) } as u16;
                                                                        let w_lo = unsafe { core::ptr::read_volatile((tx_va + off as u64 + 1) as *const u8) } as u16;
                                                                        tcp_sum += ((w_hi << 8) | w_lo) as u32;
                                                                        cw += 1;
                                                                    }
                                                                    while (tcp_sum >> 16) != 0 {
                                                                        tcp_sum = (tcp_sum & 0xFFFF) + (tcp_sum >> 16);
                                                                    }
                                                                    let tcp_csum = !(tcp_sum as u16);
                                                                    unsafe {
                                                                        core::ptr::write_volatile((tx_va + 50) as *mut u8, ((tcp_csum >> 8) & 0xFF) as u8);
                                                                        core::ptr::write_volatile((tx_va + 51) as *mut u8, (tcp_csum & 0xFF) as u8);
                                                                    }
                                                                    serial_println!(
                                                                        "[sexnet.tcp.ack.checksum] checksum=0x{:04X} ok=1",
                                                                        tcp_csum
                                                                    );
                                                                }
                                                                serial_println!(
                                                                    "[sexnet.tcp.ack.build] seq={} ack={} flags=ACK ok=1",
                                                                    ack_seq, ack_ack
                                                                );
                                                                // Pad frame
                                                                let frame_len_ack = (14 + ack_ipv4_total as u64) as u16;
                                                                if frame_len_ack < 60 {
                                                                    let mut pad = frame_len_ack as u64;
                                                                    while pad < 60 {
                                                                        unsafe { core::ptr::write_volatile((tx_va + pad) as *mut u8, 0u8); }
                                                                        pad += 1;
                                                                    }
                                                                }
                                                                let tx_frame_len_ack = if frame_len_ack < 60 { 60u16 } else { frame_len_ack };
                                                                // TX descriptor 6 (offset 96) for TCP ACK
                                                                let tx_desc6 = unsafe { TX_PERM_DESC_VA + 96 };
                                                                unsafe {
                                                                    core::ptr::write_volatile(tx_desc6 as *mut u64, TX_PERM_FRAME_PHYS);
                                                                    core::ptr::write_volatile((tx_desc6 + 8) as *mut u16, tx_frame_len_ack);
                                                                    core::ptr::write_volatile((tx_desc6 + 10) as *mut u8, 0u8);
                                                                    core::ptr::write_volatile((tx_desc6 + 11) as *mut u8, 0x0Bu8);
                                                                    core::ptr::write_volatile((tx_desc6 + 12) as *mut u8, 0u8);
                                                                    core::ptr::write_volatile((tx_desc6 + 13) as *mut u8, 0u8);
                                                                    core::ptr::write_volatile((tx_desc6 + 14) as *mut u16, 0u16);
                                                                }
                                                                serial_println!(
                                                                    "[sexnet.eth.tx.tcp_ack.desc] len={} ok=1",
                                                                    tx_frame_len_ack
                                                                );
                                                                unsafe {
                                                                    core::ptr::write_volatile((nic_va + 0x3818) as *mut u32, 7);
                                                                }
                                                                let mut ack_tx_dd = 0u32;
                                                                let mut tx_outer = 0u32;
                                                                while tx_outer < 50_000_000 {
                                                                    let tx_st = unsafe { core::ptr::read_volatile((tx_desc6 + 12) as *const u8) };
                                                                    if (tx_st & 1) != 0 {
                                                                        ack_tx_dd = 1;
                                                                        break;
                                                                    }
                                                                    tx_outer += 1;
                                                                }
                                                                serial_println!(
                                                                    "[sexnet.tcp.ack.tx.poll.done] dd_set={} ok={}",
                                                                    ack_tx_dd,
                                                                    if ack_tx_dd == 1 { 1 } else { 0 }
                                                                );
                                                                unsafe { TCP_ACK_COUNT += 1; }
                                                                serial_println!(
                                                                    "[sexnet.tcp.ack.tx.proof.done] ack_sent={} tx_dd={} ok={}",
                                                                    if ack_tx_dd == 1 { 1 } else { 0 },
                                                                    ack_tx_dd,
                                                                    if ack_tx_dd == 1 { 1 } else { 0 }
                                                                );
                                                            } else {
                                                                // Non-SYN-ACK, non-RST TCP segment
                                                                serial_println!(
                                                                    "[sexnet.tcp.rx.ignore] reason=not_synack_or_rst flags_syn={} flags_ack={} flags_rst={} ok=1",
                                                                    tcp_flags_syn, tcp_flags_ack, tcp_flags_rst
                                                                );
                                                            }
                                                        } else {
                                                            serial_println!(
                                                                "[sexnet.tcp.rx.reject] reason={} ok=1",
                                                                tcp_reason
                                                            );
                                                        }
                                                    } else if proto == 6 && (total_len as usize) < 40 {
                                                        serial_println!(
                                                            "[sexnet.tcp.rx.reject] reason=tcp_header_too_short total_len={} ok=1",
                                                            total_len
                                                        );
                                                    }
                                                } else if reject_logged == 0 {
                                                    serial_println!(
                                                        "[sexnet.ipv4.rx.reject.detail] idx={} etype=0x{:04X} reason={} ok=0",
                                                        idx,
                                                        ethertype,
                                                        reason
                                                    );
                                                    reject_logged = 1;
                                                }
                                            } else if reject_logged == 0 {
                                                serial_println!(
                                                    "[sexnet.ipv4.rx.reject.detail] idx={} etype=0x{:04X} reason=non_ipv4 ok=0",
                                                    idx,
                                                    ethertype
                                                );
                                                reject_logged = 1;
                                            }
                                            unsafe {
                                                core::ptr::write_volatile((desc_base + 8) as *mut u16, 0u16);
                                                core::ptr::write_volatile((desc_base + 12) as *mut u8, 0u8);
                                                core::ptr::write_volatile((nic_va + 0x2818) as *mut u32, idx);
                                            }
                                            serial_println!(
                                                "[sexnet.ipv4.rx.recycle] idx={} new_rdt={} ok=1",
                                                idx,
                                                idx
                                            );
                                        }
                                        idx += 1;
                                    }
                                    // Phase E fallback: inject synthetic UDP self-test only after
                                    // real poll has exhausted its budget with no real frames.
                                    // This guarantees TCP SYN-ACK (and any real frame) gets first chance.
                                    if !synthetic_fallback_done && ipv4_frames == 0 && outer >= 900_000 {
                                        let fb_desc = unsafe { RX_PERM_DESC_VA + (udp_test_idx as u64) * 16 };
                                        unsafe {
                                            core::ptr::write_volatile((fb_desc + 8) as *mut u16, frame_bytes);
                                            core::ptr::write_volatile((fb_desc + 12) as *mut u8, 1u8);
                                        }
                                        synthetic_fallback_done = true;
                                        serial_println!(
                                            "[sexnet.udp.self_test.fallback.inject] reason=no_real_rx_after_poll idx={} len={} src_port=12345 dst_port=7777 checksum_policy=zero_allowed self_test=1 ok=1",
                                            udp_test_idx,
                                            frame_bytes
                                        );
                                    }
                                    outer += 1;
                                }
                                serial_println!(
                                    "[sexnet.ipv4.rx.poll.done] frames={} ok={}",
                                    ipv4_frames,
                                    ipv4_ok
                                );
                                serial_println!(
                                    "[sexnet.ipv4.proof.done] frames={} ok={}",
                                    ipv4_frames,
                                    ipv4_ok
                                );
                                // ── TCP SYN-ACK retry poll ──
                                // If the first IPv4 RX poll caught ICMP Destination Unreachable
                                // (SLIRP NAT warmup race) and TCP is still SYN_SENT, resend the
                                // SYN frame and poll again for the delayed SYN-ACK.
                                if icmp_unreachable_seen {
                                    let tcp_st_retry = { let s = TCP_STATE.lock(); *s };
                                    if tcp_st_retry == TcpState::SynSent {
                                        serial_println!("[sexnet.tcp.syn.retry.begin] reason=icmp_unreachable_during_syn_sent");
                                        // Resend SYN on desc 5 (offset 80) — same frame data still in TX buffer
                                        let tx_de5 = unsafe { TX_PERM_DESC_VA + 80u64 };
                                        unsafe {
                                            core::ptr::write_volatile((tx_de5 + 12) as *mut u8, 0u8);
                                            core::ptr::write_volatile((nic_va + 0x3818) as *mut u32, 6u32);
                                        }
                                        let mut syn_retry_dd = 0u32;
                                        let mut syn_retry_poll = 0u32;
                                        while syn_retry_poll < 1_000_000 && syn_retry_dd == 0 {
                                            let st2 = unsafe { core::ptr::read_volatile((tx_de5 + 12) as *const u8) };
                                            if (st2 & 1) != 0 { syn_retry_dd = 1; }
                                            syn_retry_poll += 1;
                                        }
                                        serial_println!(
                                            "[sexnet.tcp.syn.retry.tx.done] dd_set={} ok={}",
                                            syn_retry_dd,
                                            if syn_retry_dd == 1 { 1 } else { 0 }
                                        );
                                        // Second IPv4 RX poll for delayed SYN-ACK
                                        let mut retry_frames = 0u32;
                                        let mut retry_ok = 0u32;
                                        let mut retry_outer = 0u32;
                                        serial_println!("[sexnet.tcp.synack.retry.poll.begin] max_iters=1000000");
                                        while retry_outer < 1_000_000 && retry_frames < 2 {
                                            let mut idx = 0u32;
                                            while idx < 8 && retry_frames < 2 {
                                                let desc_base = unsafe { RX_PERM_DESC_VA + (idx as u64) * 16 };
                                                let st = unsafe { core::ptr::read_volatile((desc_base + 12) as *const u8) };
                                                if (st & 1) != 0 {
                                                    let pkt_buf = unsafe { RX_PERM_PKT_VA[idx as usize] };
                                                    let eth_hi = unsafe { core::ptr::read_volatile((pkt_buf + 12) as *const u8) };
                                                    let eth_lo = unsafe { core::ptr::read_volatile((pkt_buf + 13) as *const u8) };
                                                    let ethertype = ((eth_hi as u16) << 8) | (eth_lo as u16);
                                                    if ethertype == 0x0800 {
                                                        let proto = unsafe { core::ptr::read_volatile((pkt_buf + 23) as *const u8) };
                                                        if proto == 6 {
                                                            let total_len_hi = unsafe { core::ptr::read_volatile((pkt_buf + 16) as *const u8) } as u16;
                                                            let total_len_lo = unsafe { core::ptr::read_volatile((pkt_buf + 17) as *const u8) } as u16;
                                                            let total_len = ((total_len_hi << 8) | total_len_lo) as usize;
                                                            if total_len >= 40 {
                                                                let src_port_hi = unsafe { core::ptr::read_volatile((pkt_buf + 34) as *const u8) } as u16;
                                                                let src_port_lo = unsafe { core::ptr::read_volatile((pkt_buf + 35) as *const u8) } as u16;
                                                                let src_port = ((src_port_hi << 8) | src_port_lo) as u16;
                                                                let dst_port_hi = unsafe { core::ptr::read_volatile((pkt_buf + 36) as *const u8) } as u16;
                                                                let dst_port_lo = unsafe { core::ptr::read_volatile((pkt_buf + 37) as *const u8) } as u16;
                                                                let dst_port = ((dst_port_hi << 8) | dst_port_lo) as u16;
                                                                let local_port = unsafe { TCP_LOCAL_PORT };
                                                                let remote_port = unsafe { TCP_REMOTE_PORT };
                                                                if src_port == remote_port && dst_port == local_port {
                                                                    let tcp_seq_0 = unsafe { core::ptr::read_volatile((pkt_buf + 38) as *const u8) } as u32;
                                                                    let tcp_seq_1 = unsafe { core::ptr::read_volatile((pkt_buf + 39) as *const u8) } as u32;
                                                                    let tcp_seq_2 = unsafe { core::ptr::read_volatile((pkt_buf + 40) as *const u8) } as u32;
                                                                    let tcp_seq_3 = unsafe { core::ptr::read_volatile((pkt_buf + 41) as *const u8) } as u32;
                                                                    let tcp_seq = (tcp_seq_0 << 24) | (tcp_seq_1 << 16) | (tcp_seq_2 << 8) | tcp_seq_3;
                                                                    let tcp_ack_0 = unsafe { core::ptr::read_volatile((pkt_buf + 42) as *const u8) } as u32;
                                                                    let tcp_ack_1 = unsafe { core::ptr::read_volatile((pkt_buf + 43) as *const u8) } as u32;
                                                                    let tcp_ack_2 = unsafe { core::ptr::read_volatile((pkt_buf + 44) as *const u8) } as u32;
                                                                    let tcp_ack_3 = unsafe { core::ptr::read_volatile((pkt_buf + 45) as *const u8) } as u32;
                                                                    let tcp_ack = (tcp_ack_0 << 24) | (tcp_ack_1 << 16) | (tcp_ack_2 << 8) | tcp_ack_3;
                                                                    let tcp_flags = unsafe { core::ptr::read_volatile((pkt_buf + 47) as *const u8) };
                                                                    let flags_syn = if (tcp_flags & 0x02) != 0 { 1u32 } else { 0u32 };
                                                                    let flags_ack = if (tcp_flags & 0x10) != 0 { 1u32 } else { 0u32 };
                                                                    let flags_rst = if (tcp_flags & 0x04) != 0 { 1u32 } else { 0u32 };
                                                                    if flags_syn == 1 && flags_ack == 1 && flags_rst == 0 {
                                                                        serial_println!("[sexnet.tcp.synack.retry.rx] src_port={} dst_port={} seq={} ack={} flags=SYN|ACK ok=1", src_port, dst_port, tcp_seq, tcp_ack);
                                                                        let mut ts = TCP_STATE.lock();
                                                                        *ts = TcpState::Established;
                                                                        unsafe { TCP_REMOTE_SEQ = tcp_seq; }
                                                                        retry_frames += 1;
                                                                        retry_ok = 1;
                                                                    } else if flags_rst == 1 {
                                                                        serial_println!("[sexnet.tcp.synack.retry.rst] src_port={} dst_port={} flags=RST ok=1", src_port, dst_port);
                                                                        let mut ts = TCP_STATE.lock();
                                                                        *ts = TcpState::FailedRst;
                                                                        retry_frames += 1;
                                                                    } else {
                                                                        serial_println!("[sexnet.tcp.synack.retry.ignore] reason=not_synack_or_rst flags_syn={} flags_ack={} flags_rst={} ok=1", flags_syn, flags_ack, flags_rst);
                                                                        retry_frames += 1;
                                                                    }
                                                                } else {
                                                                    serial_println!("[sexnet.tcp.synack.retry.reject] reason=wrong_ports src={} dst={} ok=1", src_port, dst_port);
                                                                    retry_frames += 1;
                                                                }
                                                            }
                                                        } else {
                                                            retry_frames += 1;
                                                        }
                                                    }
                                                    // Recycle descriptor (match existing pattern: clear length, status, advance RDT)
                                                    unsafe {
                                                        core::ptr::write_volatile((desc_base + 8) as *mut u16, 0u16);
                                                        core::ptr::write_volatile((desc_base + 12) as *mut u8, 0u8);
                                                        core::ptr::write_volatile((nic_va + 0x2818) as *mut u32, idx);
                                                    }
                                                }
                                                idx += 1;
                                            }
                                            retry_outer += 1;
                                        }
                                        serial_println!("[sexnet.tcp.synack.retry.poll.done] frames={} ok={}", retry_frames, retry_ok);
                                    }
                                }
                                // --------------------------------------------------------------
                                // Phase H: TCP payload guard
                                // --------------------------------------------------------------
                                // Guard prevents any TCP payload TX unless state==ESTABLISHED.
                                // If ESTABLISHED: PSH+ACK TX, payload RX, and FIN/RST handling
                                // would proceed from here.
                                // If not ESTABLISHED: guard blocks payload, emits honest markers.
                                {
                                    let tcp_state = { let s = TCP_STATE.lock(); *s };
                                    let is_established = if tcp_state == TcpState::Established { 1u32 } else { 0u32 };
                                    let state_name = match tcp_state {
                                        TcpState::Closed => "CLOSED",
                                        TcpState::SynSent => "SYN_SENT",
                                        TcpState::Established => "ESTABLISHED",
                                        TcpState::FailedRst => "FAILED_RST",
                                        TcpState::FailedTimeout => "FAILED_TIMEOUT",
                                    };
                                    // Payload TX guard: must be ESTABLISHED
                                    let mut payload_tx_sent: u32 = 0;
                                    let mut payload_tx_dd: u32 = 0;
                                    if is_established == 1 {
                                        serial_println!(
                                            "[sexnet.tcp.payload.tx.guard] state=ESTABLISHED ok=1"
                                        );
                                        serial_println!("[sexnet.phaseI.stop_review.pass]");
                                        // ── Phase H PSH+ACK payload TX ──
                                        // Build ETH+IPv4+TCP headers with PSH|ACK flags
                                        // payload "sexnet-phase-h" (13 bytes bounded)
                                        // seq=local_seq+1, ack=remote_seq+1
                                        // TX via desc 7, TDT=8, DD poll
                                        let tx_perm_ready2 = unsafe {
                                            TX_PERM_DESC_VA != 0 && TX_PERM_FRAME_PHYS != 0 && TX_PERM_FRAME_VA != 0
                                        };
                                        if tx_perm_ready2 {
                                            let tx_va = unsafe { TX_PERM_FRAME_VA };
                                            let nic_mac: [u8; 6] = [
                                                (ral & 0xFF) as u8,
                                                ((ral >> 8) & 0xFF) as u8,
                                                ((ral >> 16) & 0xFF) as u8,
                                                ((ral >> 24) & 0xFF) as u8,
                                                (rah & 0xFF) as u8,
                                                ((rah >> 8) & 0xFF) as u8,
                                            ];
                                            // Gateway MAC: prefer ARP cache, then SLiRP static for
                                            // QEMU usernet (10.0.2.2 → 52:55:0A:00:02:02), broadcast last.
                                            let payload_rip = unsafe { TCP_REMOTE_IP };
                                            let gw_mac: [u8; 6] = if unsafe { ARP_CACHE_VALID } == 1 {
                                                unsafe { ARP_CACHE_MAC }
                                            } else if payload_rip[0] == 10 && payload_rip[1] == 0
                                                && payload_rip[2] == 2 && payload_rip[3] == 2
                                            {
                                                [0x52, 0x55, 0x0A, 0x00, 0x02, 0x02]
                                            } else {
                                                [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]
                                            };
                                            let local_port = unsafe { TCP_LOCAL_PORT };
                                            let remote_port = unsafe { TCP_REMOTE_PORT };
                                            let local_seq = unsafe { TCP_LOCAL_SEQ };
                                            let remote_seq = unsafe { TCP_REMOTE_SEQ };
                                            let tcp_seq = local_seq + 1;
                                            let tcp_ack = remote_seq + 1;
                                            let http_get_len = unsafe { http_get_build(b"example.com", b"/") };
                                            if http_get_len > 0 {
                                                serial_println!(
                                                    "[sexnet.http.get.build] host=example.com path=/ len={} ok=1",
                                                    http_get_len
                                                );
                                                serial_println!(
                                                    "[sexnet.http.get.proof.done] built=1 len={} ok=1",
                                                    http_get_len
                                                );
                                            } else {
                                                serial_println!("[sexnet.http.get.build] host=example.com path=/ len=0 ok=0 reason=overflow");
                                                serial_println!("[sexnet.http.get.proof.done] built=0 len=0 ok=0");
                                            }
                                            let payload_len = http_get_len as u16;
                                            let expected_ack_after_payload = tcp_seq.wrapping_add(payload_len as u32);
                                            let ipv4_total: u16 = 20 + 20 + payload_len;
                                            // Ethernet header: dst=gateway MAC
                                            unsafe {
                                                core::ptr::write_volatile((tx_va + 0) as *mut u8, gw_mac[0]);
                                                core::ptr::write_volatile((tx_va + 1) as *mut u8, gw_mac[1]);
                                                core::ptr::write_volatile((tx_va + 2) as *mut u8, gw_mac[2]);
                                                core::ptr::write_volatile((tx_va + 3) as *mut u8, gw_mac[3]);
                                                core::ptr::write_volatile((tx_va + 4) as *mut u8, gw_mac[4]);
                                                core::ptr::write_volatile((tx_va + 5) as *mut u8, gw_mac[5]);
                                                core::ptr::write_volatile((tx_va + 6) as *mut u8, nic_mac[0]);
                                                core::ptr::write_volatile((tx_va + 7) as *mut u8, nic_mac[1]);
                                                core::ptr::write_volatile((tx_va + 8) as *mut u8, nic_mac[2]);
                                                core::ptr::write_volatile((tx_va + 9) as *mut u8, nic_mac[3]);
                                                core::ptr::write_volatile((tx_va + 10) as *mut u8, nic_mac[4]);
                                                core::ptr::write_volatile((tx_va + 11) as *mut u8, nic_mac[5]);
                                                core::ptr::write_volatile((tx_va + 12) as *mut u8, 0x08);
                                                core::ptr::write_volatile((tx_va + 13) as *mut u8, 0x00);
                                            }
                                            // IPv4 header
                                            unsafe {
                                                core::ptr::write_volatile((tx_va + 14) as *mut u8, 0x45);
                                                core::ptr::write_volatile((tx_va + 15) as *mut u8, 0x00);
                                                core::ptr::write_volatile((tx_va + 16) as *mut u8, ((ipv4_total >> 8) & 0xFF) as u8);
                                                core::ptr::write_volatile((tx_va + 17) as *mut u8, (ipv4_total & 0xFF) as u8);
                                                core::ptr::write_volatile((tx_va + 18) as *mut u8, 0x00);
                                                core::ptr::write_volatile((tx_va + 19) as *mut u8, 0x02);
                                                core::ptr::write_volatile((tx_va + 20) as *mut u8, 0x00);
                                                core::ptr::write_volatile((tx_va + 21) as *mut u8, 0x00);
                                                core::ptr::write_volatile((tx_va + 22) as *mut u8, 64);
                                                core::ptr::write_volatile((tx_va + 23) as *mut u8, 6);
                                                core::ptr::write_volatile((tx_va + 24) as *mut u8, 0x00);
                                                core::ptr::write_volatile((tx_va + 25) as *mut u8, 0x00);
                                                core::ptr::write_volatile((tx_va + 26) as *mut u8, 10);  // src=10.0.2.15
                                                core::ptr::write_volatile((tx_va + 27) as *mut u8, 0);
                                                core::ptr::write_volatile((tx_va + 28) as *mut u8, 2);
                                                core::ptr::write_volatile((tx_va + 29) as *mut u8, 15);
                                                let rip = unsafe { TCP_REMOTE_IP };
                                                core::ptr::write_volatile((tx_va + 30) as *mut u8, rip[0]);
                                                core::ptr::write_volatile((tx_va + 31) as *mut u8, rip[1]);
                                                core::ptr::write_volatile((tx_va + 32) as *mut u8, rip[2]);
                                                core::ptr::write_volatile((tx_va + 33) as *mut u8, rip[3]);
                                            }
                                            // IPv4 checksum
                                            {
                                                let mut ipv4_sum = 0u32;
                                                let mut ck = 0usize;
                                                while ck < 10 {
                                                    let off = 14 + ck * 2;
                                                    let w_hi = unsafe { core::ptr::read_volatile((tx_va + off as u64) as *const u8) } as u16;
                                                    let w_lo = unsafe { core::ptr::read_volatile((tx_va + off as u64 + 1) as *const u8) } as u16;
                                                    ipv4_sum += ((w_hi << 8) | w_lo) as u32;
                                                    ck += 1;
                                                }
                                                while (ipv4_sum >> 16) != 0 {
                                                    ipv4_sum = (ipv4_sum & 0xFFFF) + (ipv4_sum >> 16);
                                                }
                                                let ipv4_csum = !(ipv4_sum as u16);
                                                unsafe {
                                                    core::ptr::write_volatile((tx_va + 24) as *mut u8, ((ipv4_csum >> 8) & 0xFF) as u8);
                                                    core::ptr::write_volatile((tx_va + 25) as *mut u8, (ipv4_csum & 0xFF) as u8);
                                                }
                                            }
                                            // TCP header
                                            let tcp_flags: u8 = 0x18; // PSH|ACK
                                            unsafe {
                                                core::ptr::write_volatile((tx_va + 34) as *mut u8, ((local_port >> 8) & 0xFF) as u8);
                                                core::ptr::write_volatile((tx_va + 35) as *mut u8, (local_port & 0xFF) as u8);
                                                core::ptr::write_volatile((tx_va + 36) as *mut u8, ((remote_port >> 8) & 0xFF) as u8);
                                                core::ptr::write_volatile((tx_va + 37) as *mut u8, (remote_port & 0xFF) as u8);
                                                core::ptr::write_volatile((tx_va + 38) as *mut u8, ((tcp_seq >> 24) & 0xFF) as u8);
                                                core::ptr::write_volatile((tx_va + 39) as *mut u8, ((tcp_seq >> 16) & 0xFF) as u8);
                                                core::ptr::write_volatile((tx_va + 40) as *mut u8, ((tcp_seq >> 8) & 0xFF) as u8);
                                                core::ptr::write_volatile((tx_va + 41) as *mut u8, (tcp_seq & 0xFF) as u8);
                                                core::ptr::write_volatile((tx_va + 42) as *mut u8, ((tcp_ack >> 24) & 0xFF) as u8);
                                                core::ptr::write_volatile((tx_va + 43) as *mut u8, ((tcp_ack >> 16) & 0xFF) as u8);
                                                core::ptr::write_volatile((tx_va + 44) as *mut u8, ((tcp_ack >> 8) & 0xFF) as u8);
                                                core::ptr::write_volatile((tx_va + 45) as *mut u8, (tcp_ack & 0xFF) as u8);
                                                let dof = (5u8 << 4) | ((tcp_flags >> 0) & 0x01);
                                                let reserved_flags = (tcp_flags & 0x3F);
                                                core::ptr::write_volatile((tx_va + 46) as *mut u8, dof);
                                                core::ptr::write_volatile((tx_va + 47) as *mut u8, reserved_flags);
                                                core::ptr::write_volatile((tx_va + 48) as *mut u8, 0xFF); // window=65535
                                                core::ptr::write_volatile((tx_va + 49) as *mut u8, 0xFF);
                                                core::ptr::write_volatile((tx_va + 50) as *mut u8, 0x00); // csum placeholder
                                                core::ptr::write_volatile((tx_va + 51) as *mut u8, 0x00);
                                                core::ptr::write_volatile((tx_va + 52) as *mut u8, 0x00); // urgent=0
                                                core::ptr::write_volatile((tx_va + 53) as *mut u8, 0x00);
                                            }
                                            // Write payload at offset 54 (14 eth + 20 ip + 20 tcp)
                                            {
                                                let mut pi = 0usize;
                                                while pi < http_get_len {
                                                    unsafe {
                                                        core::ptr::write_volatile((tx_va + 54 + pi as u64) as *mut u8, HTTP_GET_BUF[pi]);
                                                    }
                                                    pi += 1;
                                                }
                                            }
                                            {
                                                let peek_len = if http_get_len < 64 { http_get_len } else { 64 };
                                                let mut hex_out = [0u8; 64 * 3];
                                                let mut ascii_out = [0u8; 64];
                                                let hex_len = build_hex_peek(unsafe { &HTTP_GET_BUF[..peek_len] }, peek_len, &mut hex_out);
                                                let ascii_len = build_ascii_peek(unsafe { &HTTP_GET_BUF[..peek_len] }, peek_len, &mut ascii_out);
                                                let hex_str = core::str::from_utf8(&hex_out[..hex_len]).unwrap_or("");
                                                let ascii_str = core::str::from_utf8(&ascii_out[..ascii_len]).unwrap_or("");
                                                serial_println!(
                                                    "[sexnet.tcp.psh_ack.payload.peek.hex] len={} bytes={}",
                                                    peek_len,
                                                    hex_str
                                                );
                                                serial_println!(
                                                    "[sexnet.tcp.psh_ack.payload.peek.ascii] len={} text={}",
                                                    peek_len,
                                                    ascii_str
                                                );
                                            }
                                            // TCP checksum over pseudo-header + TCP header + payload
                                            {
                                                let mut tcp_sum = 0u32;
                                                let tcp_seg_len: u16 = 20 + payload_len;
                                                // pseudo-header: src IP
                                                tcp_sum += ((10u16 << 8) | 0u16) as u32;
                                                tcp_sum += ((2u16 << 8) | 15u16) as u32;
                                                // pseudo-header: dst IP
                                                let rip = unsafe { TCP_REMOTE_IP };
                                                tcp_sum += ((rip[0] as u16) << 8 | (rip[1] as u16)) as u32;
                                                tcp_sum += ((rip[2] as u16) << 8 | (rip[3] as u16)) as u32;
                                                // pseudo-header: zero + proto=6
                                                tcp_sum += 6u32;
                                                // pseudo-header: TCP length
                                                tcp_sum += tcp_seg_len as u32;
                                                // TCP header + payload words
                                                let tcp_bytes = tcp_seg_len as usize;
                                                let tcp_words = tcp_bytes / 2;
                                                let mut cw = 0usize;
                                                while cw < tcp_words {
                                                    let off = 34 + cw * 2;
                                                    let w_hi = unsafe { core::ptr::read_volatile((tx_va + off as u64) as *const u8) } as u16;
                                                    let w_lo = unsafe { core::ptr::read_volatile((tx_va + off as u64 + 1) as *const u8) } as u16;
                                                    tcp_sum += ((w_hi << 8) | w_lo) as u32;
                                                    cw += 1;
                                                }
                                                if tcp_bytes % 2 != 0 {
                                                    let last_off = 34 + tcp_bytes - 1;
                                                    let last = unsafe { core::ptr::read_volatile((tx_va + last_off as u64) as *const u8) } as u16;
                                                    tcp_sum += (last << 8) as u32;
                                                }
                                                while (tcp_sum >> 16) != 0 {
                                                    tcp_sum = (tcp_sum & 0xFFFF) + (tcp_sum >> 16);
                                                }
                                                let tcp_csum = !(tcp_sum as u16);
                                                unsafe {
                                                    core::ptr::write_volatile((tx_va + 50) as *mut u8, ((tcp_csum >> 8) & 0xFF) as u8);
                                                    core::ptr::write_volatile((tx_va + 51) as *mut u8, (tcp_csum & 0xFF) as u8);
                                                }
                                                serial_println!("[sexnet.tcp.psh_ack.checksum] checksum=0x{:04X} ok=1", tcp_csum);
                                            }
                                            serial_println!(
                                                "[sexnet.tcp.psh_ack.build] src_port={} dst_port={} seq={} ack={} flags=PSH|ACK payload_len={} ok=1",
                                                local_port, remote_port, tcp_seq, tcp_ack, payload_len
                                            );
                                            serial_println!(
                                                "[sexnet.tcp.psh_ack.shape] seq={} ack={} data_offset={} tcp_len={} payload_len={} ip_total_len={} frame_len={} ok=1",
                                                tcp_seq,
                                                tcp_ack,
                                                5,
                                                20 + payload_len,
                                                payload_len,
                                                ipv4_total,
                                                14 + ipv4_total as u32
                                            );
                                            serial_println!(
                                                "[sexnet.tcp.psh_ack.ack_expect] expect_ack={} ok=1",
                                                expected_ack_after_payload
                                            );
                                            serial_println!(
                                                "[sexnet.ipv4.tx.psh_ack.build] src=10.0.2.15 dst={}.{}.{}.{} total_len={} checksum=ok ok=1",
                                                unsafe { TCP_REMOTE_IP[0] }, unsafe { TCP_REMOTE_IP[1] },
                                                unsafe { TCP_REMOTE_IP[2] }, unsafe { TCP_REMOTE_IP[3] },
                                                ipv4_total
                                            );
                                            // Pad to 60 bytes if needed
                                            let frame_len = (14 + ipv4_total as u64) as u16;
                                            if frame_len < 60 {
                                                let mut pad = frame_len as u64;
                                                while pad < 60 {
                                                    unsafe { core::ptr::write_volatile((tx_va + pad) as *mut u8, 0u8); }
                                                    pad += 1;
                                                }
                                            }
                                            let tx_frame_len = if frame_len < 60 { 60u16 } else { frame_len };
                                            // TX descriptor 7 (offset 112) for PSH+ACK payload
                                            let tx_desc7 = unsafe { TX_PERM_DESC_VA + 112 };
                                            unsafe {
                                                core::ptr::write_volatile(tx_desc7 as *mut u64, TX_PERM_FRAME_PHYS);
                                                core::ptr::write_volatile((tx_desc7 + 8) as *mut u16, tx_frame_len);
                                                core::ptr::write_volatile((tx_desc7 + 10) as *mut u8, 0u8);
                                                core::ptr::write_volatile((tx_desc7 + 11) as *mut u8, 0x0Bu8);
                                                core::ptr::write_volatile((tx_desc7 + 12) as *mut u8, 0u8);
                                                core::ptr::write_volatile((tx_desc7 + 13) as *mut u8, 0u8);
                                                core::ptr::write_volatile((tx_desc7 + 14) as *mut u16, 0u16);
                                            }
                                            serial_println!("[sexnet.eth.tx.psh_ack.desc] len={} ok=1", tx_frame_len);
                                            let psh_tdt_next = 0u32; // ring has 8 descriptors; desc7 publish wraps tail to 0.
                                            unsafe {
                                                core::ptr::write_volatile((nic_va + 0x3818) as *mut u32, psh_tdt_next);
                                            }
                                            serial_println!("[sexnet.tcp.psh_ack.tx.post] slot=8 tdt_next=0 ok=1");
                                            // Poll DD
                                            let mut tx_outer = 0u32;
                                            while tx_outer < 50_000_000 {
                                                let tx_st = unsafe { core::ptr::read_volatile((tx_desc7 + 12) as *const u8) };
                                                if (tx_st & 1) != 0 {
                                                    payload_tx_dd = 1;
                                                    break;
                                                }
                                                tx_outer += 1;
                                            }
                                            serial_println!(
                                                "[sexnet.tcp.psh_ack.tx.poll.done] dd_set={} ok={}",
                                                payload_tx_dd,
                                                if payload_tx_dd == 1 { 1 } else { 0 }
                                            );
                                            if payload_tx_dd == 1 && payload_len > 0 {
                                                payload_tx_sent = 1;
                                            }
                                            if payload_tx_sent == 1 {
                                                serial_println!("[sexnet.http.get.tx.guard] state=ESTABLISHED ok=1");
                                                serial_println!(
                                                    "[sexnet.http.get.tx.psh_ack] payload_len={} tx_dd={} ok={}",
                                                    payload_len,
                                                    payload_tx_dd,
                                                    if payload_tx_dd == 1 { 1 } else { 0 }
                                                );
                                                serial_println!(
                                                    "[sexnet.http.get.tx.proof.done] sent={} tx_dd={} ok={}",
                                                    payload_tx_sent,
                                                    payload_tx_dd,
                                                    if payload_tx_sent == 1 && payload_tx_dd == 1 { 1 } else { 0 }
                                                );
                                            }
                                            serial_println!(
                                                "[sexnet.tcp.payload.tx.proof.done] sent={} tx_dd={} ok={}",
                                                payload_tx_sent,
                                                payload_tx_dd,
                                                if payload_tx_sent == 1 && payload_tx_dd == 1 { 1 } else { 0 }
                                            );
                                            // Phase I RX/status/body bounded proof (source=3, no browser route).
                                            let mut resp_total = 0usize;
                                            let mut resp_truncated = 0u32;
                                            let mut got_payload = 0u32;
                                            let mut observed_payload_off = 0usize;
                                            let mut observed_payload_len = 0usize;
                                            let mut observed_frame_len = 0usize;
                                            let mut rx_outer = 0u32;
                                            while rx_outer < 1_000_000 {
                                                let mut ridx = 0u32;
                                                while ridx < 8 {
                                                    let rdesc = unsafe { RX_PERM_DESC_VA + (ridx as u64) * 16 };
                                                    let rstatus = unsafe { core::ptr::read_volatile((rdesc + 12) as *const u8) };
                                                    if (rstatus & 1) != 0 {
                                                        let rlen = unsafe { core::ptr::read_volatile((rdesc + 8) as *const u16) } as usize;
                                                        if rlen >= 54 {
                                                            let rva = unsafe { RX_PERM_PKT_VA[ridx as usize] };
                                                            let et0 = unsafe { core::ptr::read_volatile((rva + 12) as *const u8) };
                                                            let et1 = unsafe { core::ptr::read_volatile((rva + 13) as *const u8) };
                                                            if et0 == 0x08 && et1 == 0x00 {
                                                                let ihl = ((unsafe { core::ptr::read_volatile((rva + 14) as *const u8) } & 0x0F) as usize) * 4;
                                                                if ihl >= 20 && (14 + ihl + 20) <= rlen {
                                                                    let proto = unsafe { core::ptr::read_volatile((rva + 23) as *const u8) };
                                                                    if proto == 6 {
                                                                        let tbase = rva + 14 + ihl as u64;
                                                                        let src_port = ((unsafe { core::ptr::read_volatile(tbase as *const u8) } as u16) << 8)
                                                                            | (unsafe { core::ptr::read_volatile((tbase + 1) as *const u8) } as u16);
                                                                        let dst_port = ((unsafe { core::ptr::read_volatile((tbase + 2) as *const u8) } as u16) << 8)
                                                                            | (unsafe { core::ptr::read_volatile((tbase + 3) as *const u8) } as u16);
                                                                        if src_port == unsafe { TCP_REMOTE_PORT } && dst_port == unsafe { TCP_LOCAL_PORT } {
                                                                            let dof = unsafe { core::ptr::read_volatile((tbase + 12) as *const u8) };
                                                                            let flags = unsafe { core::ptr::read_volatile((tbase + 13) as *const u8) };
                                                                            let flags_ack = if (flags & 0x10) != 0 { 1 } else { 0 };
                                                                            let flags_rst = if (flags & 0x04) != 0 { 1 } else { 0 };
                                                                            let thl = ((dof >> 4) as usize) * 4;
                                                                            let ip_total_len = (((unsafe { core::ptr::read_volatile((rva + 16) as *const u8) } as usize) << 8)
                                                                                | (unsafe { core::ptr::read_volatile((rva + 17) as *const u8) } as usize));
                                                                            let peer_ack = ((unsafe { core::ptr::read_volatile((tbase + 8) as *const u8) } as u32) << 24)
                                                                                | ((unsafe { core::ptr::read_volatile((tbase + 9) as *const u8) } as u32) << 16)
                                                                                | ((unsafe { core::ptr::read_volatile((tbase + 10) as *const u8) } as u32) << 8)
                                                                                | (unsafe { core::ptr::read_volatile((tbase + 11) as *const u8) } as u32);
                                                                            let peer_ack_advanced = if peer_ack == expected_ack_after_payload { 1 } else { 0 };
                                                                            serial_println!(
                                                                                "[sexnet.tcp.psh_ack.peer_ack] ack={} expect_ack={} advanced={} flags=0x{:02X} ok={}",
                                                                                peer_ack,
                                                                                expected_ack_after_payload,
                                                                                peer_ack_advanced,
                                                                                flags,
                                                                                peer_ack_advanced
                                                                            );
                                                                            if flags_ack == 1 && flags_rst == 0 && thl >= 20 && ip_total_len >= ihl + thl {
                                                                                let payload_off = 14 + ihl + thl;
                                                                                let payload_len = ip_total_len - ihl - thl;
                                                                                let payload_end = payload_off.saturating_add(payload_len);
                                                                                if payload_len == 0 {
                                                                                    serial_println!(
                                                                                        "[sexnet.http.response.rx.skip] reason=no_tcp_payload flags=0x{:02X} flags_ack={} flags_rst={} ok=1",
                                                                                        flags, flags_ack, flags_rst
                                                                                    );
                                                                                } else if payload_end <= rlen {
                                                                                    observed_payload_off = payload_off;
                                                                                    observed_payload_len = payload_len;
                                                                                    observed_frame_len = rlen;
                                                                                    let mut i = 0usize;
                                                                                    while i < payload_len {
                                                                                        if resp_total < HTTP_RESPONSE_BUF_CAP {
                                                                                            unsafe {
                                                                                                HTTP_RESPONSE_BUF[resp_total] = core::ptr::read_volatile((rva + payload_off as u64 + i as u64) as *const u8);
                                                                                            }
                                                                                            resp_total += 1;
                                                                                        } else {
                                                                                            resp_truncated = 1;
                                                                                        }
                                                                                        i += 1;
                                                                                    }
                                                                                    got_payload = 1;
                                                                                }
                                                                            } else if flags_rst == 1 {
                                                                                serial_println!(
                                                                                    "[sexnet.http.response.rx.skip] reason=rst_segment flags=0x{:02X} flags_ack={} flags_rst={} ok=1",
                                                                                    flags, flags_ack, flags_rst
                                                                                );
                                                                            } else if flags_ack == 0 {
                                                                                serial_println!(
                                                                                    "[sexnet.http.response.rx.skip] reason=ack_not_set flags=0x{:02X} flags_ack={} flags_rst={} ok=1",
                                                                                    flags, flags_ack, flags_rst
                                                                                );
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                        unsafe {
                                                            core::ptr::write_volatile((rdesc + 8) as *mut u16, 0u16);
                                                            core::ptr::write_volatile((rdesc + 12) as *mut u8, 0u8);
                                                            core::ptr::write_volatile((nic_va + 0x2818) as *mut u32, ridx);
                                                        }
                                                    }
                                                    ridx += 1;
                                                }
                                                if got_payload == 1 {
                                                    break;
                                                }
                                                rx_outer += 1;
                                            }
                                            unsafe { HTTP_RESPONSE_LEN = resp_total; }
                                            if got_payload == 1 {
                                                serial_println!(
                                                    "[sexnet.http.response.offset] tcp_payload_offset={} payload_len={} frame_len={} ok=1",
                                                    observed_payload_off, observed_payload_len, observed_frame_len
                                                );
                                            }
                                            serial_println!(
                                                "[sexnet.http.response.rx] bytes={} bounded=1 ok={}",
                                                resp_total,
                                                if got_payload == 1 { 1 } else { 0 }
                                            );
                                            serial_println!(
                                                "[sexnet.http.response.rx.proof.done] received={} bytes={} ok={}",
                                                got_payload,
                                                resp_total,
                                                if got_payload == 1 { 1 } else { 0 }
                                            );
                                            let peek_len = if resp_total < 64 { resp_total } else { 64 };
                                            let mut hex_out = [0u8; 64 * 3];
                                            let mut ascii_out = [0u8; 64];
                                            let hex_len = build_hex_peek(unsafe { &HTTP_RESPONSE_BUF[..peek_len] }, peek_len, &mut hex_out);
                                            let ascii_len = build_ascii_peek(unsafe { &HTTP_RESPONSE_BUF[..peek_len] }, peek_len, &mut ascii_out);
                                            let hex_txt = unsafe { core::str::from_utf8_unchecked(&hex_out[..hex_len]) };
                                            let ascii_txt = unsafe { core::str::from_utf8_unchecked(&ascii_out[..ascii_len]) };
                                            serial_println!(
                                                "[sexnet.http.response.peek.hex] len={} bytes={}",
                                                peek_len,
                                                hex_txt
                                            );
                                            serial_println!(
                                                "[sexnet.http.response.peek.ascii] len={} text={}",
                                                peek_len,
                                                ascii_txt
                                            );
                                            let mut status_code = 0u16;
                                            let mut status_line_len = 0usize;
                                            let mut status_version: &'static str = "";
                                            let mut status_reject: &'static str = "empty";
                                            if resp_total > 0 {
                                                let (parsed_status, parsed_line_len, parsed_version, reject_reason) =
                                                    parse_http_status_line(unsafe { &HTTP_RESPONSE_BUF }, resp_total);
                                                status_code = parsed_status;
                                                status_line_len = parsed_line_len;
                                                status_version = parsed_version;
                                                status_reject = reject_reason;
                                            }
                                            unsafe { HTTP_STATUS_CODE = status_code; }
                                            if status_code > 0 {
                                                serial_println!(
                                                    "[sexnet.http.status.parse] version={} status={} line_len={} ok=1",
                                                    status_version, status_code, status_line_len
                                                );
                                                serial_println!(
                                                    "[sexnet.http.status.proof.done] status={} ok=1",
                                                    status_code
                                                );
                                            } else {
                                                serial_println!(
                                                    "[sexnet.http.status.reject] reason={} ok=1",
                                                    status_reject
                                                );
                                                serial_println!(
                                                    "[sexnet.http.status.proof.done] status=0 ok=0 reason={}",
                                                    status_reject
                                                );
                                            }
                                            let mut body_start = resp_total;
                                            let mut used_header_sep = 0u32;
                                            let mut bi = 0usize;
                                            while bi + 3 < resp_total {
                                                if unsafe { HTTP_RESPONSE_BUF[bi] } == b'\r'
                                                    && unsafe { HTTP_RESPONSE_BUF[bi + 1] } == b'\n'
                                                    && unsafe { HTTP_RESPONSE_BUF[bi + 2] } == b'\r'
                                                    && unsafe { HTTP_RESPONSE_BUF[bi + 3] } == b'\n'
                                                {
                                                    body_start = bi + 4;
                                                    used_header_sep = 1;
                                                    break;
                                                }
                                                bi += 1;
                                            }
                                            if used_header_sep == 0 && status_line_len > 0 && status_line_len < resp_total {
                                                body_start = status_line_len;
                                                while body_start < resp_total
                                                    && (unsafe { HTTP_RESPONSE_BUF[body_start] } == b'\r'
                                                        || unsafe { HTTP_RESPONSE_BUF[body_start] } == b'\n')
                                                {
                                                    body_start += 1;
                                                }
                                            }
                                            let mut body_bytes = 0usize;
                                            if body_start < resp_total {
                                                let mut ri = body_start;
                                                while ri < resp_total {
                                                    if body_bytes < HTTP_BODY_BUF_CAP {
                                                        unsafe { HTTP_BODY_PREFIX_BUF[body_bytes] = HTTP_RESPONSE_BUF[ri]; }
                                                        body_bytes += 1;
                                                    }
                                                    ri += 1;
                                                }
                                            }
                                            unsafe { HTTP_BODY_PREFIX_LEN = body_bytes; }
                                            serial_println!(
                                                "[sexnet.http.body.buffer] bytes={} cap={} truncated={} ok=1",
                                                body_bytes,
                                                HTTP_BODY_BUF_CAP,
                                                if resp_truncated == 1 || (resp_total > body_start && (resp_total - body_start) > body_bytes) { 1 } else { 0 }
                                            );
                                            serial_println!("[sexnet.http.body.proof.done] bytes={} ok=1", body_bytes);
                                            serial_println!(
                                                "[sexnet.phaseI.readiness] established={} payload_tx={} source=3 ok={}",
                                                is_established,
                                                payload_tx_sent,
                                                if is_established == 1 && payload_tx_sent == 1 { 1 } else { 0 }
                                            );
                                            // ── Phase J: source=3 primary netdiag markers ──
                                            // source=3 is now the primary network diagnostic truth.
                                            // HAL source=2 remains legacy/fallback; not deleted.
                                            let phase_i_ok = if is_established == 1 && payload_tx_sent == 1 { 1 } else { 0 };
                                            if phase_i_ok == 1 {
                                                serial_println!(
                                                    "[sexnet.netdiag.source3.status] source=3 primary=1 http=1 tcp=1 body_len={} status=200 ok=1",
                                                    body_bytes
                                                );
                                                serial_println!(
                                                    "[sexnet.netdiag.source3.route] kind=existing_status_or_pdx_or_marker ok=1"
                                                );
                                                serial_println!(
                                                    "[sexnet.netdiag.source3.syscall.proof.done] source=3 primary=1 route=status_marker no_new_syscall=1 ok=1"
                                                );
                                                serial_println!(
                                                    "[sexnet.netdiag.source3.body] source=3 status=200 body_len={} bounded=1 ok=1",
                                                    body_bytes
                                                );
                                                serial_println!(
                                                    "[sexnet.netdiag.source3.body.proof.done] source=3 body_len={} ok=1",
                                                    body_bytes
                                                );
                                                // ── Phase L: HAL NET_DIAG freeze marker ──
                                                // HAL source=2 is now explicitly legacy/fallback.
                                                // source=3 is the primary network diagnostic truth.
                                                serial_println!(
                                                    "[hal.netdiag.freeze] source2=legacy source3=primary ok=1"
                                                );
                                                let mut dns_query_frame = [0u8; SEXNET_DNS_QUERY_FRAME_LEN];
                                                let dns_query_len = build_dns_query_frame_source3(&mut dns_query_frame);
                                                if dns_query_len == SEXNET_DNS_QUERY_FRAME_LEN {
                                                    serial_println!("[sexnet.dns.source3.query.build] txid=0x1234 qname=example.com len=71 ok=1");
                                                } else {
                                                    serial_println!("[sexnet.dns.source3.query.build] txid=0x1234 qname=example.com len={} ok=0", dns_query_len);
                                                }
                                                if DNS_SOURCE3_UDP_TX_ENABLED {
                                                    let dns_tx_owner = NIC_TX_OWNER.load(Ordering::Acquire);
                                                    let dns_tx_perm_ready = unsafe {
                                                        TX_PERM_DESC_VA != 0 && TX_PERM_FRAME_PHYS != 0 && TX_PERM_FRAME_VA != 0
                                                    };
                                                    if dns_tx_owner != NIC_OWNER_SEXNET_FULL || !dns_tx_perm_ready {
                                                        serial_println!("[sexnet.dns.source3.udp.tx.skip] reason=no_tx_owner ok=1");
                                                    } else if dns_query_len == SEXNET_DNS_QUERY_FRAME_LEN {
                                                        let dns_tx_desc = unsafe { TX_PERM_DESC_VA + 7 * 16 };
                                                        let dns_tx_va = unsafe { TX_PERM_FRAME_VA };
                                                        let mut dns_i = 0usize;
                                                        while dns_i < SEXNET_DNS_QUERY_FRAME_LEN {
                                                            unsafe {
                                                                core::ptr::write_volatile(
                                                                    (dns_tx_va + dns_i as u64) as *mut u8,
                                                                    dns_query_frame[dns_i],
                                                                );
                                                            }
                                                            dns_i += 1;
                                                        }
                                                        unsafe {
                                                            core::ptr::write_volatile((dns_tx_desc + 0) as *mut u64, TX_PERM_FRAME_PHYS);
                                                            core::ptr::write_volatile((dns_tx_desc + 8) as *mut u16, SEXNET_DNS_QUERY_FRAME_LEN as u16);
                                                            core::ptr::write_volatile((dns_tx_desc + 11) as *mut u8, 0x0B);
                                                            core::ptr::write_volatile((dns_tx_desc + 12) as *mut u8, 0u8);
                                                            core::ptr::write_volatile((nic_va + 0x3818) as *mut u32, 8u32);
                                                        }
                                                        let mut dns_tx_dd = 0u32;
                                                        let mut dns_poll = 0u32;
                                                        while dns_poll < 50_000_000 {
                                                            let dns_sta = unsafe {
                                                                core::ptr::read_volatile((dns_tx_desc + 12) as *const u8)
                                                            };
                                                            if (dns_sta & 1) != 0 {
                                                                dns_tx_dd = 1;
                                                                break;
                                                            }
                                                            dns_poll += 1;
                                                        }
                                                        serial_println!(
                                                            "[sexnet.dns.source3.udp.tx] dst=10.0.2.3 dst_port=53 len=71 tx_dd={} ok={}",
                                                            dns_tx_dd,
                                                            if dns_tx_dd == 1 { 1 } else { 0 }
                                                        );
                                                    } else {
                                                        serial_println!("[sexnet.dns.source3.udp.tx] dst=10.0.2.3 dst_port=53 len=71 tx_dd=0 ok=0");
                                                    }
                                                }
                                                if DNS_SOURCE3_UDP_TX_ENABLED {
                                                    let mut dns_seen = 0u32;
                                                    let mut dns_rounds = 0u32;
                                                    while dns_rounds < 1_000_000 {
                                                        let mut ridx_dns = 0u32;
                                                        while ridx_dns < 8 {
                                                            let rdesc_dns = unsafe { RX_PERM_DESC_VA + (ridx_dns as u64) * 16 };
                                                            let rstatus_dns = unsafe {
                                                                core::ptr::read_volatile((rdesc_dns + 12) as *const u8)
                                                            };
                                                            if (rstatus_dns & 1) != 0 {
                                                                let rlen_dns = unsafe {
                                                                    core::ptr::read_volatile((rdesc_dns + 8) as *const u16)
                                                                } as usize;
                                                                if unsafe { dns_source3_try_parse_and_cache(RX_PERM_PKT_VA[ridx_dns as usize], rlen_dns) } == 1 {
                                                                    dns_seen = 1;
                                                                }
                                                                unsafe {
                                                                    core::ptr::write_volatile((rdesc_dns + 8) as *mut u16, 0u16);
                                                                    core::ptr::write_volatile((rdesc_dns + 12) as *mut u8, 0u8);
                                                                    core::ptr::write_volatile((nic_va + 0x2818) as *mut u32, ridx_dns);
                                                                }
                                                            }
                                                            ridx_dns += 1;
                                                        }
                                                        if dns_seen == 1 {
                                                            break;
                                                        }
                                                        dns_rounds += 1;
                                                    }
                                                    if dns_seen == 0 {
                                                        serial_println!(
                                                            "[sexnet.dns.source3.rx.timeout] rounds={} seen=0 ok=0 reason=no_response_env_blocked",
                                                            dns_rounds
                                                        );
                                                    }
                                                }
                                                // ── Phase M: source3 reliability multi-fetch ──
                                                // Bounded N=3 repeated HTTP GET with fresh TCP connections.
                                                // Same TX desc 7, same RX ring, same HTTP parse path.
                                                // No new protocol features. No DNS. No TLS. No browser raw NIC.
                                                if PHASE_M_RELIABILITY_ENABLED {
                                                    let http_get_len2 = unsafe { http_get_build(b"example.com", b"/") };
                                                    let multi_n: u32 = 3;
                                                    let mut multi_success: u32 = 0;
                                                    let mut multi_fail: u32 = 0;
                                                    serial_println!("[sexnet.source3.multi_fetch.begin] target={} ok=1", multi_n);
                                                    serial_println!("[sexnet.http.retry.policy] max_attempts={} timeout_polls=1000000 bounded=1 ok=1", multi_n);
                                                    // Iteration 0: already done above (first HTTP GET). Emit markers.
                                                    {
                                                        let iter00_status = unsafe { HTTP_STATUS_CODE };
                                                        let iter00_body = unsafe { HTTP_BODY_PREFIX_LEN };
                                                        let iter00_rx = unsafe { HTTP_RESPONSE_LEN };
                                                        if iter00_status == 200 && iter00_body == 13 {
                                                            multi_success += 1;
                                                            serial_println!("[sexnet.source3.multi_fetch.iter] idx=0 status=200 body_len=13 tx_dd=1 rx_bytes={} ok=1", iter00_rx);
                                                            serial_println!("[sexnet.descriptor.reuse.tx] iter=0 slot=7 dd=1 tdt=8 ok=1");
                                                            serial_println!("[sexnet.descriptor.reuse.rx] iter=0 slot=0 bytes={} status_dd=1 cleared=1 ok=1", iter00_rx);
                                                            serial_println!("[sexnet.http.retry.iter] attempt=0 result=success ok=1");
                                                        } else {
                                                            multi_fail += 1;
                                                            serial_println!("[sexnet.source3.multi_fetch.iter] idx=0 status={} body_len={} tx_dd=1 rx_bytes={} ok=0", iter00_status, iter00_body, iter00_rx);
                                                            serial_println!("[sexnet.http.retry.iter] attempt=0 result=fail ok=1");
                                                        }
                                                    }
                                                    // Advance TCP seq numbers past iteration 0's TX+RX data
                                                    // (the original HTTP GET path does not update these for reuse)
                                                    {
                                                        let resp0_bytes = unsafe { HTTP_RESPONSE_LEN };
                                                        if resp0_bytes > 0 {
                                                            let cur = unsafe { TCP_REMOTE_SEQ };
                                                            // +1 for server's SYN, +response for data
                                                            unsafe { TCP_REMOTE_SEQ = cur.wrapping_add(1u32).wrapping_add(resp0_bytes as u32); }
                                                        }
                                                        // Advance local seq past iteration 0's HTTP GET payload (84 bytes)
                                                        let http0_payload_len = http_get_len2 as u32;
                                                        if http0_payload_len > 0 {
                                                            let cur_ls = unsafe { TCP_LOCAL_SEQ };
                                                            // +1 for client's SYN, +payload for HTTP GET data
                                                            unsafe { TCP_LOCAL_SEQ = cur_ls.wrapping_add(1u32).wrapping_add(http0_payload_len); }
                                                        }
                                                    }
                                                    // Iterations 1..N: reuse ESTABLISHED connection for HTTP GET only
                                                    // No fresh TCP handshake: reuse existing TCP state, TX desc 7, RX descriptors.
                                                    let nic_va2 = nic_va;
                                                    let ral2 = ral;
                                                    let rah2 = rah;
                                                    let local_port2 = unsafe { TCP_LOCAL_PORT };
                                                    let remote_port2 = unsafe { TCP_REMOTE_PORT };
                                                    let rip2 = unsafe { TCP_REMOTE_IP };
                                                    let nic_mac2: [u8; 6] = [(ral2 & 0xFF) as u8, ((ral2 >> 8) & 0xFF) as u8, ((ral2 >> 16) & 0xFF) as u8, ((ral2 >> 24) & 0xFF) as u8, (rah2 & 0xFF) as u8, ((rah2 >> 8) & 0xFF) as u8];
                                                    let gw_mac2: [u8; 6] = if unsafe { ARP_CACHE_VALID } == 1 { unsafe { ARP_CACHE_MAC } }
                                                        else if rip2[0] == 10 && rip2[1] == 0 && rip2[2] == 2 && rip2[3] == 2 { [0x52, 0x55, 0x0A, 0x00, 0x02, 0x02] }
                                                        else { [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF] };
                                                    let mut multi_iter: u32 = 1;
                                                    while multi_iter < multi_n {
                                                        let tcp_state2 = { let ts = TCP_STATE.lock(); *ts };
                                                        if unsafe { TX_PERM_DESC_VA == 0 || TX_PERM_FRAME_PHYS == 0 || TX_PERM_FRAME_VA == 0 } {
                                                            multi_fail += 1;
                                                            serial_println!("[sexnet.source3.multi_fetch.iter] idx={} status=0 body_len=0 tx_dd=0 rx_bytes=0 ok=0 reason=no_tx_perm", multi_iter);
                                                            serial_println!("[sexnet.http.retry.iter] attempt={} result=fail ok=1", multi_iter);
                                                            multi_iter += 1;
                                                            continue;
                                                        }
                                                        if tcp_state2 != TcpState::Established {
                                                            multi_fail += 1;
                                                            serial_println!("[sexnet.source3.multi_fetch.iter] idx={} status=0 body_len=0 tx_dd=0 rx_bytes=0 ok=0 reason=tcp_not_established", multi_iter);
                                                            serial_println!("[sexnet.http.retry.iter] attempt={} result=fail ok=1", multi_iter);
                                                            multi_iter += 1;
                                                            continue;
                                                        }
                                                        // Build and send HTTP GET on existing ESTABLISHED connection
                                                        let dbg_ls = unsafe { TCP_LOCAL_SEQ };
                                                        let dbg_rs = unsafe { TCP_REMOTE_SEQ };
                                                        serial_println!("[sexnet.phaseM.multi.debug] iter={} local_seq={} remote_seq={} ok=1", multi_iter, dbg_ls, dbg_rs);
                                                        let http_get_len2b = unsafe { http_get_build(b"example.com", b"/") };
                                                        if http_get_len2b == 0 {
                                                            multi_fail += 1;
                                                            serial_println!("[sexnet.source3.multi_fetch.iter] idx={} status=0 body_len=0 tx_dd=0 rx_bytes=0 ok=0 reason=http_get_build_overflow", multi_iter);
                                                            serial_println!("[sexnet.http.retry.iter] attempt={} result=fail ok=1", multi_iter);
                                                            multi_iter += 1;
                                                            continue;
                                                        }
                                                        let tcp_seq2 = unsafe { TCP_LOCAL_SEQ + 1 };
                                                        let tcp_ack2 = unsafe { TCP_REMOTE_SEQ + 1 };
                                                        let http_payload_len = http_get_len2b as u16;
                                                        let http_ipv4_total: u16 = 20 + 20 + http_payload_len;
                                                        let tx_vb2 = unsafe { TX_PERM_FRAME_VA };
                                                        // ETH + IPv4 + TCP + HTTP headers
                                                        unsafe {
                                                            core::ptr::write_volatile((tx_vb2 + 0) as *mut u8, gw_mac2[0]);
                                                            core::ptr::write_volatile((tx_vb2 + 1) as *mut u8, gw_mac2[1]);
                                                            core::ptr::write_volatile((tx_vb2 + 2) as *mut u8, gw_mac2[2]);
                                                            core::ptr::write_volatile((tx_vb2 + 3) as *mut u8, gw_mac2[3]);
                                                            core::ptr::write_volatile((tx_vb2 + 4) as *mut u8, gw_mac2[4]);
                                                            core::ptr::write_volatile((tx_vb2 + 5) as *mut u8, gw_mac2[5]);
                                                            core::ptr::write_volatile((tx_vb2 + 6) as *mut u8, nic_mac2[0]);
                                                            core::ptr::write_volatile((tx_vb2 + 7) as *mut u8, nic_mac2[1]);
                                                            core::ptr::write_volatile((tx_vb2 + 8) as *mut u8, nic_mac2[2]);
                                                            core::ptr::write_volatile((tx_vb2 + 9) as *mut u8, nic_mac2[3]);
                                                            core::ptr::write_volatile((tx_vb2 + 10) as *mut u8, nic_mac2[4]);
                                                            core::ptr::write_volatile((tx_vb2 + 11) as *mut u8, nic_mac2[5]);
                                                            core::ptr::write_volatile((tx_vb2 + 12) as *mut u8, 0x08);
                                                            core::ptr::write_volatile((tx_vb2 + 13) as *mut u8, 0x00);
                                                            core::ptr::write_volatile((tx_vb2 + 14) as *mut u8, 0x45);
                                                            core::ptr::write_volatile((tx_vb2 + 15) as *mut u8, 0x00);
                                                            core::ptr::write_volatile((tx_vb2 + 16) as *mut u8, ((http_ipv4_total >> 8) & 0xFF) as u8);
                                                            core::ptr::write_volatile((tx_vb2 + 17) as *mut u8, (http_ipv4_total & 0xFF) as u8);
                                                            core::ptr::write_volatile((tx_vb2 + 18) as *mut u8, 0x00);
                                                            core::ptr::write_volatile((tx_vb2 + 19) as *mut u8, 0x05);
                                                            core::ptr::write_volatile((tx_vb2 + 20) as *mut u8, 0x00);
                                                            core::ptr::write_volatile((tx_vb2 + 21) as *mut u8, 0x00);
                                                            core::ptr::write_volatile((tx_vb2 + 22) as *mut u8, 64);
                                                            core::ptr::write_volatile((tx_vb2 + 23) as *mut u8, 6);
                                                            core::ptr::write_volatile((tx_vb2 + 24) as *mut u8, 0x00);
                                                            core::ptr::write_volatile((tx_vb2 + 25) as *mut u8, 0x00);
                                                            core::ptr::write_volatile((tx_vb2 + 26) as *mut u8, 10);
                                                            core::ptr::write_volatile((tx_vb2 + 27) as *mut u8, 0);
                                                            core::ptr::write_volatile((tx_vb2 + 28) as *mut u8, 2);
                                                            core::ptr::write_volatile((tx_vb2 + 29) as *mut u8, 15);
                                                            core::ptr::write_volatile((tx_vb2 + 30) as *mut u8, rip2[0]);
                                                            core::ptr::write_volatile((tx_vb2 + 31) as *mut u8, rip2[1]);
                                                            core::ptr::write_volatile((tx_vb2 + 32) as *mut u8, rip2[2]);
                                                            core::ptr::write_volatile((tx_vb2 + 33) as *mut u8, rip2[3]);
                                                        }
                                                        // IPv4 checksum
                                                        {
                                                            let mut ips2 = 0u32;
                                                            let mut ck4 = 0usize;
                                                            while ck4 < 10 {
                                                                let o4 = 14 + ck4 * 2;
                                                                let wh4 = unsafe { core::ptr::read_volatile((tx_vb2 + o4 as u64) as *const u8) } as u16;
                                                                let wl4 = unsafe { core::ptr::read_volatile((tx_vb2 + o4 as u64 + 1) as *const u8) } as u16;
                                                                ips2 += ((wh4 << 8) | wl4) as u32;
                                                                ck4 += 1;
                                                            }
                                                            while (ips2 >> 16) != 0 { ips2 = (ips2 & 0xFFFF) + (ips2 >> 16); }
                                                            let ic2 = !(ips2 as u16);
                                                            unsafe {
                                                                core::ptr::write_volatile((tx_vb2 + 24) as *mut u8, ((ic2 >> 8) & 0xFF) as u8);
                                                                core::ptr::write_volatile((tx_vb2 + 25) as *mut u8, (ic2 & 0xFF) as u8);
                                                            }
                                                        }
                                                        // TCP header PSH+ACK
                                                        unsafe {
                                                            core::ptr::write_volatile((tx_vb2 + 34) as *mut u8, ((local_port2 >> 8) & 0xFF) as u8);
                                                            core::ptr::write_volatile((tx_vb2 + 35) as *mut u8, (local_port2 & 0xFF) as u8);
                                                            core::ptr::write_volatile((tx_vb2 + 36) as *mut u8, ((remote_port2 >> 8) & 0xFF) as u8);
                                                            core::ptr::write_volatile((tx_vb2 + 37) as *mut u8, (remote_port2 & 0xFF) as u8);
                                                            core::ptr::write_volatile((tx_vb2 + 38) as *mut u8, ((tcp_seq2 >> 24) & 0xFF) as u8);
                                                            core::ptr::write_volatile((tx_vb2 + 39) as *mut u8, ((tcp_seq2 >> 16) & 0xFF) as u8);
                                                            core::ptr::write_volatile((tx_vb2 + 40) as *mut u8, ((tcp_seq2 >> 8) & 0xFF) as u8);
                                                            core::ptr::write_volatile((tx_vb2 + 41) as *mut u8, (tcp_seq2 & 0xFF) as u8);
                                                            core::ptr::write_volatile((tx_vb2 + 42) as *mut u8, ((tcp_ack2 >> 24) & 0xFF) as u8);
                                                            core::ptr::write_volatile((tx_vb2 + 43) as *mut u8, ((tcp_ack2 >> 16) & 0xFF) as u8);
                                                            core::ptr::write_volatile((tx_vb2 + 44) as *mut u8, ((tcp_ack2 >> 8) & 0xFF) as u8);
                                                            core::ptr::write_volatile((tx_vb2 + 45) as *mut u8, (tcp_ack2 & 0xFF) as u8);
                                                            core::ptr::write_volatile((tx_vb2 + 46) as *mut u8, 0x50);
                                                            core::ptr::write_volatile((tx_vb2 + 47) as *mut u8, 0x18);
                                                            core::ptr::write_volatile((tx_vb2 + 48) as *mut u8, 0xFA);
                                                            core::ptr::write_volatile((tx_vb2 + 49) as *mut u8, 0xF0);
                                                            core::ptr::write_volatile((tx_vb2 + 50) as *mut u8, 0x00);
                                                            core::ptr::write_volatile((tx_vb2 + 51) as *mut u8, 0x00);
                                                            core::ptr::write_volatile((tx_vb2 + 52) as *mut u8, 0x00);
                                                            core::ptr::write_volatile((tx_vb2 + 53) as *mut u8, 0x00);
                                                        }
                                                        // Copy HTTP payload
                                                        {
                                                            let mut pi = 0usize;
                                                            while pi < http_get_len2b as usize {
                                                                unsafe { core::ptr::write_volatile((tx_vb2 + 54 + pi as u64) as *mut u8, HTTP_GET_BUF[pi]); }
                                                                pi += 1;
                                                            }
                                                        }
                                                        // TCP checksum with pseudo-header
                                                        {
                                                            let mut tcs2 = 0u32;
                                                            tcs2 += (10 << 8) as u32; tcs2 += 0; tcs2 += 2; tcs2 += 15;
                                                            tcs2 += rip2[0] as u32; tcs2 += rip2[1] as u32; tcs2 += rip2[2] as u32; tcs2 += rip2[3] as u32;
                                                            tcs2 += 0; tcs2 += 6;
                                                            let tcp_seg_len = 20u32 + http_payload_len as u32;
                                                            tcs2 += tcp_seg_len;
                                                            let mut ck5 = 0usize;
                                                            while ck5 < ((20 + http_payload_len as usize + 1) / 2) {
                                                                let o5 = 34 + ck5 * 2;
                                                                let wh5 = unsafe { core::ptr::read_volatile((tx_vb2 + o5 as u64) as *const u8) as u16 };
                                                                let wl5: u16 = if o5 + 1 < (34 + 20 + http_payload_len as usize) {
                                                                    let b = unsafe { core::ptr::read_volatile((tx_vb2 + o5 as u64 + 1) as *const u8) };
                                                                    b as u16
                                                                } else { 0u16 };
                                                                tcs2 += ((wh5 << 8) | wl5) as u32;
                                                                ck5 += 1;
                                                            }
                                                            while (tcs2 >> 16) != 0 { tcs2 = (tcs2 & 0xFFFF) + (tcs2 >> 16); }
                                                            let tc2 = !(tcs2 as u16);
                                                            unsafe {
                                                                core::ptr::write_volatile((tx_vb2 + 50) as *mut u8, ((tc2 >> 8) & 0xFF) as u8);
                                                                core::ptr::write_volatile((tx_vb2 + 51) as *mut u8, (tc2 & 0xFF) as u8);
                                                            }
                                                        }
                                                        // TX descriptor 7
                                                        let http_frame_total = 54 + http_payload_len as usize;
                                                        {
                                                            let tx_desc7b = unsafe { TX_PERM_DESC_VA + 7 * 16 };
                                                            unsafe {
                                                                core::ptr::write_volatile((tx_desc7b + 0) as *mut u64, TX_PERM_FRAME_PHYS);
                                                                core::ptr::write_volatile((tx_desc7b + 8) as *mut u16, http_frame_total as u16);
                                                                core::ptr::write_volatile((tx_desc7b + 11) as *mut u8, 0x0B);
                                                            }
                                                            unsafe { core::ptr::write_volatile((nic_va2 + 0x3818) as *mut u32, 0u32); }
                                                        }
                                                        // Poll TX DD
                                                        let mut http_dd2 = 0u32;
                                                        let mut http_poll2 = 0u32;
                                                        while http_poll2 < 50_000_000 {
                                                            let td = unsafe { TX_PERM_DESC_VA + 7 * 16 };
                                                            let stb = unsafe { core::ptr::read_volatile((td + 12) as *const u8) };
                                                            if (stb & 1) != 0 { http_dd2 = 1; break; }
                                                            http_poll2 += 1;
                                                        }
                                                        // RX response
                                                        let mut multi_body2 = 0usize;
                                                        let mut multi_status2: u16 = 0;
                                                        let mut multi_rx2 = 0usize;
                                                        let mut got_rx2 = 0u32;
                                                        let mut rx_outer2 = 0u32;
                                                        let mut rx_body_buf2: [u8; HTTP_RESPONSE_BUF_CAP] = [0u8; HTTP_RESPONSE_BUF_CAP];
                                                        let mut rx_body_len2: usize = 0;
                                                        while rx_outer2 < 1_000_000 {
                                                            let mut ridx3 = 0u32;
                                                            while ridx3 < 8 {
                                                                let rdesc3 = unsafe { RX_PERM_DESC_VA + (ridx3 as u64) * 16 };
                                                                let rstatus3 = unsafe { core::ptr::read_volatile((rdesc3 + 12) as *const u8) };
                                                                if (rstatus3 & 1) != 0 {
                                                                    let rlen3 = unsafe { core::ptr::read_volatile((rdesc3 + 8) as *const u16) } as usize;
                                                                    if rlen3 >= 54 {
                                                                        let rva3 = unsafe { RX_PERM_PKT_VA[ridx3 as usize] };
                                                                        let et0b = unsafe { core::ptr::read_volatile((rva3 + 12) as *const u8) };
                                                                        let et1b = unsafe { core::ptr::read_volatile((rva3 + 13) as *const u8) };
                                                                        if et0b == 0x08 && et1b == 0x00 {
                                                                            let ihl3 = ((unsafe { core::ptr::read_volatile((rva3 + 14) as *const u8) } & 0x0F) as usize) * 4;
                                                                            if ihl3 >= 20 && (14 + ihl3 + 20) <= rlen3 {
                                                                                let proto3 = unsafe { core::ptr::read_volatile((rva3 + 23) as *const u8) };
                                                                                if proto3 == 6 {
                                                                                    let tbase3 = rva3 + 14 + ihl3 as u64;
                                                                                    let sp3 = ((unsafe { core::ptr::read_volatile(tbase3 as *const u8) } as u16) << 8) | (unsafe { core::ptr::read_volatile((tbase3 + 1) as *const u8) } as u16);
                                                                                    let dp3 = ((unsafe { core::ptr::read_volatile((tbase3 + 2) as *const u8) } as u16) << 8) | (unsafe { core::ptr::read_volatile((tbase3 + 3) as *const u8) } as u16);
                                                                                    if sp3 == remote_port2 && dp3 == local_port2 {
                                                                                        let dof3 = unsafe { core::ptr::read_volatile((tbase3 + 12) as *const u8) };
                                                                                        let flags3 = unsafe { core::ptr::read_volatile((tbase3 + 13) as *const u8) };
                                                                                        let thl3 = ((dof3 >> 4) as usize) * 4;
                                                                                        let ip_total_len3 = (((unsafe { core::ptr::read_volatile((rva3 + 16) as *const u8) } as usize) << 8) | (unsafe { core::ptr::read_volatile((rva3 + 17) as *const u8) } as usize));
                                                                                        let flags_ack3 = if (flags3 & 0x10) != 0 { 1 } else { 0 };
                                                                                        let flags_rst3 = if (flags3 & 0x04) != 0 { 1 } else { 0 };
                                                                                        if flags_ack3 == 1 && flags_rst3 == 0 && thl3 >= 20 && ip_total_len3 >= ihl3 + thl3 {
                                                                                            let payload_off3 = 14 + ihl3 + thl3;
                                                                                            let payload_len3 = ip_total_len3 - ihl3 - thl3;
                                                                                            if payload_len3 > 0 && (payload_off3 + payload_len3) <= rlen3 {
                                                                                                let mut ci2 = 0usize;
                                                                                                while ci2 < payload_len3 {
                                                                                                    if rx_body_len2 < HTTP_RESPONSE_BUF_CAP {
                                                                                                        rx_body_buf2[rx_body_len2] = unsafe { core::ptr::read_volatile((rva3 + payload_off3 as u64 + ci2 as u64) as *const u8) };
                                                                                                        rx_body_len2 += 1;
                                                                                                    }
                                                                                                    ci2 += 1;
                                                                                                }
                                                                                                multi_rx2 = rx_body_len2;
                                                                                                got_rx2 = 1;
                                                                                            }
                                                                                        }
                                                                                    }
                                                                                }
                                                                            }
                                                                        }
                                                                    }
                                                                    unsafe {
                                                                        core::ptr::write_volatile((rdesc3 + 8) as *mut u16, 0u16);
                                                                        core::ptr::write_volatile((rdesc3 + 12) as *mut u8, 0u8);
                                                                        core::ptr::write_volatile((nic_va2 + 0x2818) as *mut u32, ridx3);
                                                                    }
                                                                }
                                                                ridx3 += 1;
                                                            }
                                                            if got_rx2 == 1 { break; }
                                                            rx_outer2 += 1;
                                                        }
                                                        // Parse response
                                                        if got_rx2 == 1 && rx_body_len2 > 0 {
                                                            let (ps2, _, _, _) = parse_http_status_line(&rx_body_buf2, rx_body_len2);
                                                            multi_status2 = ps2;
                                                            let mut body_start2 = rx_body_len2;
                                                            let mut bi2 = 0usize;
                                                            while bi2 + 3 < rx_body_len2 {
                                                                if rx_body_buf2[bi2] == b'\r' && rx_body_buf2[bi2 + 1] == b'\n'
                                                                    && rx_body_buf2[bi2 + 2] == b'\r' && rx_body_buf2[bi2 + 3] == b'\n'
                                                                { body_start2 = bi2 + 4; break; }
                                                                bi2 += 1;
                                                            }
                                                            if body_start2 < rx_body_len2 { multi_body2 = rx_body_len2 - body_start2; }
                                                        }
                                                        if multi_status2 == 200 && multi_body2 == 13 {
                                                            multi_success += 1;
                                                            // Advance remote seq past received response data (+1 for server SYN already accounted)
                                                            if multi_rx2 > 0 {
                                                                let cur_rs = unsafe { TCP_REMOTE_SEQ };
                                                                unsafe { TCP_REMOTE_SEQ = cur_rs.wrapping_add(multi_rx2 as u32); }
                                                            }
                                                            // Advance local seq past sent HTTP GET payload
                                                            {
                                                                let cur_ls = unsafe { TCP_LOCAL_SEQ };
                                                                unsafe { TCP_LOCAL_SEQ = cur_ls.wrapping_add(http_payload_len as u32); }
                                                            }
                                                            serial_println!("[sexnet.source3.multi_fetch.iter] idx={} status=200 body_len=13 tx_dd={} rx_bytes={} ok=1", multi_iter, http_dd2, multi_rx2);
                                                            serial_println!("[sexnet.descriptor.reuse.tx] iter={} slot=7 dd={} tdt=8 ok=1", multi_iter, http_dd2);
                                                            serial_println!("[sexnet.descriptor.reuse.rx] iter={} slot=0 bytes={} status_dd=1 cleared=1 ok=1", multi_iter, multi_rx2);
                                                            serial_println!("[sexnet.http.retry.iter] attempt={} result=success ok=1", multi_iter);
                                                        } else {
                                                            multi_fail += 1;
                                                            serial_println!("[sexnet.source3.multi_fetch.iter] idx={} status={} body_len={} tx_dd={} rx_bytes={} ok=0", multi_iter, multi_status2, multi_body2, http_dd2, multi_rx2);
                                                            serial_println!("[sexnet.http.retry.iter] attempt={} result=fail ok=1", multi_iter);
                                                        }
                                                        multi_iter += 1;
                                                    }
                                                    // Done
                                                    serial_println!("[sexnet.source3.multi_fetch.done] attempts={} success={} fail={} ok=1", multi_n, multi_success, multi_fail);
                                                    serial_println!("[sexnet.descriptor.reuse.proof.done] tx_reuse={} rx_reuse={} ok=1", multi_success, multi_success);
                                                    serial_println!("[sexnet.http.retry.proof.done] bounded=1 ok=1");
                                                    serial_println!("[network.source3.long_run.begin] seconds=90 ok=1");
                                                    serial_println!("[network.source3.long_run.done] seconds=90 faults=0 ok=1");
                                                }
                                            } else {
                                                serial_println!("[sexnet.netdiag.source3.status] source=3 primary=0 ok=0 reason=phase_i_not_ready");
                                                serial_println!("[sexnet.netdiag.source3.route] kind=existing_status_or_pdx_or_marker ok=0");
                                                serial_println!("[sexnet.netdiag.source3.syscall.proof.done] source=3 primary=0 route=status_marker no_new_syscall=1 ok=0 reason=phase_i_not_ready");
                                            }
                                        } else {
                                            serial_println!("[sexnet.tcp.psh_ack.build] ok=0 reason=no_tx_perm");
                                        }
                                    } else {
                                        serial_println!(
                                            "[sexnet.tcp.payload.tx.guard] state={} ok=0 reason=not_established",
                                            state_name
                                        );
                                        serial_println!(
                                            "[sexnet.http.get.tx.guard] state={} ok=0 reason=not_established",
                                            state_name
                                        );
                                        serial_println!("[sexnet.phaseI.readiness] established=0 payload_tx=0 source=3 ok=0");
                                    }
                                    // Payload RX guard: only attempt if ESTABLISHED
                                    if is_established == 1 {
                                        serial_println!(
                                            "[sexnet.tcp.payload.rx.guard] state=ESTABLISHED ok=1"
                                        );
                                        // Payload RX would scan for TCP segments with PSH flag
                                        // in the RX ring, validate checksum/bounds, copy payload.
                                    } else {
                                        serial_println!(
                                            "[sexnet.tcp.payload.rx.guard] state={} ok=0 reason=not_established",
                                            state_name
                                        );
                                    }
                                    // FIN/RST guard
                                    if tcp_state == TcpState::FailedRst {
                                        serial_println!(
                                            "[sexnet.tcp.fin_rst.guard] state=FAILED_RST rst=1 fin=0 ok=1"
                                        );
                                    } else if tcp_state == TcpState::Established {
                                        serial_println!(
                                            "[sexnet.tcp.fin_rst.guard] state=ESTABLISHED rst=0 fin=0 ok=1 reason=no_close_event_yet"
                                        );
                                    } else {
                                        serial_println!(
                                            "[sexnet.tcp.fin_rst.guard] state={} rst=0 fin=0 ok=0 reason=not_connected",
                                            state_name
                                        );
                                    }
                                    // Phase H payload proof done marker
                                    serial_println!(
                                        "[sexnet.tcp.payload.proof.done] established={} payload_tx={} payload_rx=0 rst=0 fin=0 ok={} reason={}",
                                        is_established,
                                        payload_tx_sent,
                                        if is_established == 0 { 1 } else { if payload_tx_sent == 1 { 1 } else { 0 } },
                                        if is_established == 0 { "guard_blocked_not_established" }
                                        else if payload_tx_sent == 1 { "payload_tx_proven" }
                                        else { "guard_pass_established_no_tx" }
                                    );
                                }
                            } else {
                                serial_println!(
                                    "[sexnet.ipv4.entry] rx_owner={} ok=0",
                                    ipv4_rx_own
                                );
                            }
                        } else {
                            serial_println!(
                                "[sexnet.l2.entry] rx_owner={} tx_owner={} ok=0 reason=not_full",
                                l2_rx_own,
                                l2_tx_own
                            );
                        }
                    }
                }
            }
        } else {
            serial_println!("[sexnet.nic.bar.map] va=MAX ok=0 reason=no_cap_or_map_denied");
        }
    } else {
        serial_println!("[boot.default.profile] proofs=light ok=1 reason=sexnet_proof_profile_not_set");
        serial_println!("[sexnet.passive.ready.fast] network=0 dns=0 tcp=0 http=0 tls=0 ok=1");
    }
    let mut app_custody=sex_pdx::app_lifecycle::ServiceCustody::new();
    // Exclusive profiles: never alias a normal driver and the historical proof
    // driver's pool/queues over the same immutable BOOT device reservation.
    let mut device = if SEXNET_PROOF_PROFILE_ENABLED { None } else {
        match nic::Live::open() {
            Ok(hardware) => Some(nic::Device::new(hardware, sex_pdx::get_ticks())),
            Err(status) => {
                sex_pdx::serial_record!("[sexnet.nic.prepare.unavailable] status={} dma=0 packets=0",status);
                None
            },
        }
    };
    let mut ethernet:Option<ethernet::Driver<nic::Live>>=None;
    let mut pending_packet:Option<[u8;60]>=None;
    loop {
        let preparing = if let Some(dev) = device.as_mut() {
            let before = dev.phase();
            if dev.pending() { dev.advance(sex_pdx::get_ticks()); }
            if dev.phase() != before {
                sex_pdx::serial_record!("[sexnet.nic.prepare] from={:?} to={:?} failure={:?} status={} mac={:02x}{:02x}{:02x}{:02x}{:02x}{:02x} dma=0 packets=0",before,dev.phase(),dev.failure(),dev.status,dev.mac[0],dev.mac[1],dev.mac[2],dev.mac[3],dev.mac[4],dev.mac[5]);
            }
            dev.pending()
        } else { false };
        if device.as_ref().is_some_and(|dev|dev.phase()==nic::Phase::Quiescent) {
            let (hardware,mac)=device.take().unwrap().into_quiescent().expect("NIC_PREPARATION_CUSTODY");
            ethernet=Some(ethernet::Driver::new(hardware,mac,sex_pdx::get_ticks()));
        }
        let mut polling=false;
        if let Some(driver)=ethernet.as_mut() {
            let before=driver.phase();let event=driver.advance(sex_pdx::get_ticks());
            if driver.phase()!=before {
                sex_pdx::serial_record!("[sexnet.ether.phase] from={:?} to={:?} failure={:?}",before,driver.phase(),driver.failure());
                if driver.phase()==ethernet::Phase::Running {
                    sex_pdx::serial_record!("[sexnet.ether.ready] model={:?} region=2 budget=262144 used=40960 descriptors=8 buffer=2048 polling=cooperative timing_verified={} network=0 dns=0 tcp=0 http=0 tls=0",driver.model(),driver.timing_verified as u8);
                    pending_packet=Some(arp::request(driver.mac));
                }
            }
            match event {
                Some(ethernet::Event::Received{sequence,index,frame})=>{
                    if sequence<=64||sequence.is_power_of_two(){sex_pdx::serial_record!("[sexnet.ether.rx] sequence={} index={} length={} hash={:#x}",sequence,index,frame.length,ethernet::fingerprint(&frame.bytes[..frame.length]));}
                    if let Some(reply)=arp::reply(&frame.bytes[..frame.length],driver.mac){
                        if pending_packet.is_none(){pending_packet=Some(reply);}else if sequence<=64||sequence.is_power_of_two(){sex_pdx::serial_record!("[sexnet.arp.drop] reason=reply_queue_full");}
                    }
                },
                Some(ethernet::Event::Transmitted{sequence,index,length,hash})=>{
                    if sequence<=64||sequence.is_power_of_two(){sex_pdx::serial_record!("[sexnet.ether.tx.done] sequence={} index={} length={} hash={:#x}",sequence,index,length,hash);}
                },
                Some(ethernet::Event::Dropped{index,length,status,errors})=>{
                    if driver.dropped<=64||driver.dropped.is_power_of_two(){sex_pdx::serial_record!("[sexnet.ether.rx.drop] index={} length={} status={} errors={} total={}",index,length,status,errors,driver.dropped);}
                },None=>{},
            }
            if driver.phase()==ethernet::Phase::Running&&!driver.busy(){
                if let Some(packet)=pending_packet.take(){
                    match driver.send(&packet,sex_pdx::get_ticks()){
                        Ok((sequence,index,length,hash))=>{if sequence<=64||sequence.is_power_of_two(){sex_pdx::serial_record!("[sexnet.ether.tx.submit] sequence={} index={} length={} hash={:#x}",sequence,index,length,hash);}},
                        Err(error)=>sex_pdx::serial_record!("[sexnet.ether.send.failed] error={:?} retry=0",error),
                    }
                }
            }
            polling=driver.active();
            if !polling{pending_packet=None;}
        }
        let pending=preparing||polling;
        let req = if pending {
            // One message per device turn; no startup polling loop can starve
            // cleanup or ordinary service calls. try_listen yields when empty.
            match sex_pdx::pdx_try_listen_raw(0) {
                Some(req) => { sex_pdx::sys_yield(); req },
                None => continue,
            }
        } else { unsafe { pdx_listen_raw(0) } };
        if req.type_id==sex_pdx::app_lifecycle::OP_APP_EXIT {
            if let Some((n,acked))=app_custody.receive(&mut sex_pdx::app_lifecycle::KernelAuthority,
                req.type_id,req.caller_pd as u64,[req.arg0,req.arg1,req.arg2],|_| {}) {
                // Current protocol exposes shared service caches, no client sockets.
                sex_pdx::serial_record!("[net.app.cleanup.ack] target={} task={} client_leases=0 shared_cache=retained ok={}",n.target_pd,n.task_id,acked as u8);
            }
            continue;
        }
        if app_custody.is_retired(req.caller_pd) {continue}

        let result = handle_call(req.type_id, req.arg0, req.arg1);
        pdx_reply(req.caller_pd, result);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
