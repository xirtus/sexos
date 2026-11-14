//! Intel firmware loading against a synthetic .sfi and a scripted controller that
//! checks every command, plus (when `SEXOS_BT_FIRMWARE_DIR` holds the files from
//! scripts/fetch_bt_firmware.sh) the real linux-firmware images.
use sex_bt::hci::{self, Event};
use sex_bt::intel::{self, Loader, Outcome, Version};

fn cmd(opcode: u16, plen: usize, fill: u8) -> Vec<u8> {
    let mut c = opcode.to_le_bytes().to_vec();
    c.push(plen as u8);
    c.extend(std::iter::repeat(fill).take(plen));
    c
}

/// A synthetic .sfi: RSA header (and ECDSA header when `ecdsa`), then a command buffer
/// with a Write Boot Params at boot address 0x0024_1234 and a fragment over 252 bytes.
fn sfi(ecdsa: bool) -> (Vec<u8>, usize) {
    let mut f = vec![0x11u8; 644];
    f[0] = 0x06;
    f[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    if ecdsa {
        let mut e = vec![0x22u8; 320];
        e[0] = 0x06;
        e[8..12].copy_from_slice(&0x0002_0000u32.to_le_bytes());
        f.extend_from_slice(&e);
    }
    let start = f.len();
    f.extend(cmd(0xfc8e, 5, 0xa1)); // 8 bytes: one fragment
    let mut boot = 0xfc0eu16.to_le_bytes().to_vec();
    boot.push(4);
    boot.extend_from_slice(&0x0024_1234u32.to_le_bytes()); // 7 bytes
    f.extend(boot);
    f.extend(cmd(0xfc8e, 2, 0xa2)); // 7 + 5 = 12: second fragment
    f.extend(cmd(0xfc8e, 255, 0xa3)); // 258
    f.extend(cmd(0xfc8e, 3, 0xa4)); // 258 + 6 = 264: split 252 + 12
    (f, start)
}

const DDC: [u8; 9] = [0x03, 0x28, 0x01, 0x18, 0x04, 0x29, 0x01, 0x03, 0x00];

fn tlv_version(image_type: u8, sbe: u8) -> Vec<u8> {
    let mut r = vec![0u8];
    let mut put = |t: u8, v: &[u8]| { r.push(t); r.push(v.len() as u8); r.extend_from_slice(v); };
    put(0x10, &0x0000_0410u32.to_le_bytes()); // CNVi top: type 0x410, step 0
    put(0x11, &0x0000_0410u32.to_le_bytes()); // CNVR top
    put(0x12, &0x0017_0100u32.to_le_bytes()); // CNVi BT: hw variant 0x17 (TyP)
    put(0x1c, &[image_type]);
    put(0x1f, &0x0000_9a2bu32.to_le_bytes());
    put(0x2f, &[sbe]);
    put(0x30, &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]); // unknown types are skipped
    r
}

/// A controller in its bootloader: answers each command, routes answers over bulk
/// when the command came over bulk (as USB parts do), and logs what it got.
struct Fake { version: Vec<u8>, out: Vec<Vec<u8>>, log: Vec<(u16, bool, u8, usize)>, payload: usize, want_payload: usize, boot: Option<u32>, ddc: Vec<Vec<u8>> }

impl Fake {
    fn take(&mut self, o: &intel::Out) {
        let (opcode, params) = hci::parse_command(&o.packet).unwrap();
        self.log.push((opcode, o.bulk, params.first().copied().unwrap_or(0), params.len()));
        let bulk = |e: Vec<u8>| { let mut a = vec![hci::ACL]; a.extend_from_slice(&e[1..]); a };
        match opcode {
            intel::OP_READ_VERSION => { assert_eq!(params, [0xff]); self.out.push(hci::command_complete(opcode, &self.version)); }
            intel::OP_SECURE_SEND => {
                assert!(o.bulk && params.len() <= 253);
                if params[0] == 0x01 { self.payload += params.len() - 1; }
                self.out.push(bulk(hci::command_complete(opcode, &[0])));
                if self.payload == self.want_payload { self.out.push(bulk(hci::event(0xff, &[0x06, 0x00, 0x09, 0xfc, 0x00]))); }
            }
            intel::OP_RESET => {
                assert!(!o.bulk);
                assert_eq!(&params[..4], &[0, 1, 0, 1]);
                self.boot = Some(u32::from_le_bytes(params[4..8].try_into().unwrap()));
                self.out.push(hci::event(0xff, &[0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]));
            }
            intel::OP_DDC_WRITE => { self.ddc.push(params.to_vec()); self.out.push(hci::command_complete(opcode, &[0, 0x28, 0x01])); }
            _ => panic!("unexpected command {opcode:#06x}"),
        }
    }
}

fn run(version: Vec<u8>, file: &[u8], payload_start: usize) -> (Loader, Fake) {
    let mut l = Loader::new();
    let mut f = Fake { version, out: vec![], log: vec![], payload: 0, want_payload: file.len() - payload_start, boot: None, ddc: vec![] };
    for _ in 0..1000 {
        if let Some((sfi_name, ddc_name)) = l.wants_firmware() {
            assert_eq!((sfi_name.as_str(), ddc_name.as_str()), ("ibt-0041-0041.sfi", "ibt-0041-0041.ddc"));
            l.provide(file, Some(&DDC)).unwrap();
        }
        if let Some(o) = l.next() { f.take(&o); }
        for p in std::mem::take(&mut f.out) {
            let p = intel::bootloader_event(&p).unwrap_or(p);
            l.on_event(&hci::parse_event(&p).unwrap()).unwrap();
        }
        if l.outcome().is_some() { break; }
    }
    (l, f)
}

#[test]
fn tlv_rsa_load_sequence() {
    let (file, start) = sfi(true);
    let (l, f) = run(tlv_version(0x01, 0x00), &file, start);
    assert_eq!(l.outcome(), Some(Outcome::Loaded));
    assert_eq!(f.boot, Some(0x0024_1234));
    // (opcode, bulk, fragment type, parameter length) of every command, in order.
    let expect: Vec<(u16, bool, u8, usize)> = vec![
        (0xfc05, false, 0xff, 1),
        (0xfc09, true, 0x00, 129),
        (0xfc09, true, 0x03, 253), (0xfc09, true, 0x03, 5),
        (0xfc09, true, 0x02, 253), (0xfc09, true, 0x02, 5),
        (0xfc09, true, 0x01, 9), (0xfc09, true, 0x01, 13),
        (0xfc09, true, 0x01, 253), (0xfc09, true, 0x01, 13),
        (0xfc01, false, 0x00, 8),
        (0xfc8b, false, 0x03, 4), (0xfc8b, false, 0x04, 5),
    ];
    assert_eq!(f.log, expect);
    assert_eq!(f.ddc, vec![DDC[..4].to_vec(), DDC[4..].to_vec()]);
}

#[test]
fn tlv_ecdsa_headers() {
    let (file, start) = sfi(true);
    let (l, f) = run(tlv_version(0x01, 0x01), &file, start);
    assert_eq!(l.outcome(), Some(Outcome::Loaded));
    let headers: Vec<_> = f.log.iter().filter(|e| e.0 == 0xfc09 && e.2 != 0x01).map(|e| (e.2, e.3)).collect();
    assert_eq!(headers, [(0x00, 129), (0x03, 97), (0x02, 97)]);
}

#[test]
fn already_operational_loads_nothing() {
    let (file, start) = sfi(true);
    let (l, f) = run(tlv_version(0x03, 0x00), &file, start);
    assert_eq!(l.outcome(), Some(Outcome::AlreadyRunning));
    assert_eq!(f.log.len(), 1);
}

#[test]
fn legacy_version_and_names() {
    // AX201 in its bootloader: platform 0x37, variant 0x13 (HrP), revision 0, fw variant 6, fw revision 4.
    let v = Version::parse(&[0, 0x37, 0x13, 0x00, 0x06, 0x04, 0x10, 0x25, 0x14, 0x00]).unwrap();
    assert!(v.bootloader());
    assert_eq!(v.firmware_base().unwrap(), "ibt-19-0-4");
    let op = Version::parse(&[0, 0x37, 0x13, 0x00, 0x23, 0x04, 0x10, 0x25, 0x14, 0x00]).unwrap();
    assert!(!op.bootloader());
    // A legacy part takes the RSA headers only; the payload starts at 644.
    let (file, _) = sfi(false);
    let (cmds, boot) = intel::secure_send_plan(&file, &v).unwrap();
    assert_eq!(boot, 0x0024_1234);
    assert_eq!(cmds.len(), 1 + 2 + 2 + 4);
    // An AX210 file needs its ECDSA header.
    let tlv = Version::parse(&tlv_version(1, 0)).unwrap();
    assert_eq!(tlv.hw_variant(), 0x17);
    assert!(intel::secure_send_plan(&file, &tlv).is_err());
    // AX211 naming: CNVi 0x400 -> 0040.
    let mut ax211 = tlv_version(1, 0);
    ax211[3..7].copy_from_slice(&0x0000_0400u32.to_le_bytes());
    assert_eq!(Version::parse(&ax211).unwrap().firmware_base().unwrap(), "ibt-0040-0041");
    assert_eq!(intel::part(0x8087, 0x0032).unwrap().tlv, true);
    assert_eq!(intel::part(0x8087, 0x0026).unwrap().tlv, false);
    assert!(Version::parse(&[0x01]).is_err());
}

#[test]
fn damaged_firmware_is_refused() {
    let v = Version::parse(&tlv_version(1, 0)).unwrap();
    let (mut file, _) = sfi(true);
    file.truncate(file.len() - 2);
    assert!(intel::secure_send_plan(&file, &v).is_err());
    assert!(intel::ddc_commands(&[0x05, 0x01]).is_err());
    let mut l = Loader::new();
    l.next();
    l.on_event(&hci::parse_event(&hci::command_complete(0xfc05, &tlv_version(1, 0))).unwrap()).unwrap();
    assert!(l.wants_firmware().is_some());
    assert!(l.provide(&file, None).is_err());
    assert!(l.failed());
    // A failed download result ends the load.
    let (good, _) = sfi(true);
    let mut l = Loader::new();
    l.next();
    l.on_event(&hci::parse_event(&hci::command_complete(0xfc05, &tlv_version(1, 0))).unwrap()).unwrap();
    l.provide(&good, None).unwrap();
    assert!(l.on_event(&Event::Vendor { params: &[0x06, 0x01, 0x09, 0xfc, 0x00] }).is_err());
}

#[test]
fn real_firmware_when_fetched() {
    let Ok(dir) = std::env::var("SEXOS_BT_FIRMWARE_DIR") else { eprintln!("SEXOS_BT_FIRMWARE_DIR unset: real images skipped"); return };
    let dir = std::path::Path::new(&dir);
    for sbe in [0u8, 1] {
        let v = Version::parse(&tlv_version(1, sbe)).unwrap();
        let sfi = std::fs::read(dir.join(format!("{}.sfi", v.firmware_base().unwrap()))).unwrap();
        let (cmds, boot) = intel::secure_send_plan(&sfi, &v).unwrap();
        assert!(cmds.len() > 1000 && boot != 0, "ax210 sbe={sbe} cmds={} boot={boot:#x}", cmds.len());
    }
    let v = Version::parse(&[0, 0x37, 0x13, 0x00, 0x06, 0x04, 0x10, 0x25, 0x14, 0x00]).unwrap();
    let sfi = std::fs::read(dir.join("ibt-19-0-4.sfi")).unwrap();
    let (cmds, boot) = intel::secure_send_plan(&sfi, &v).unwrap();
    assert!(cmds.len() > 1000 && boot != 0);
    for name in ["ibt-19-0-4.ddc", "ibt-0041-0041.ddc", "ibt-0040-0041.ddc"] {
        assert!(!intel::ddc_commands(&std::fs::read(dir.join(name)).unwrap()).unwrap().is_empty());
    }
}
