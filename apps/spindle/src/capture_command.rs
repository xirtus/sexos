//! Person-triggered whole-desktop PNG, using one held read-only screenshot bank.
//! The main loop owns both replies and the Files region for the complete job.
use crate::Scrollback;
use sex_graphics::png_encode::Encoder;
use sex_pdx::{fs as f, screen_capture as c};
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Announce,
    Frame,
    Stat,
    Create,
    Write,
    Sync,
    Release,
    Fenced,
}
pub struct Capture {
    phase: Phase,
    bank: Option<c::Bank>,
    header: Option<c::Header>,
    encoder: Option<Encoder>,
    sequence: u32,
    epoch: u64,
    task: u32,
    started: u64,
    sent_at: u64,
    pending: bool,
    to_files: bool,
    file_seq: u32,
    path: [u8; 512],
    path_len: usize,
    offset: u64,
    chunk: usize,
    lease: bool,
    revoke_sent: bool,
    cancelled: bool,
    saved: bool,
}
pub static mut CAPTURE: Capture = Capture::new();
impl Capture {
    pub const fn new() -> Self {
        Self {
            phase: Phase::Idle,
            bank: None,
            header: None,
            encoder: None,
            sequence: 0,
            epoch: 0,
            task: 0,
            started: 0,
            sent_at: 0,
            pending: false,
            to_files: false,
            file_seq: 0x900000,
            path: [0; 512],
            path_len: 0,
            offset: 0,
            chunk: 0,
            lease: false,
            revoke_sent: false,
            cancelled: false,
            saved: false,
        }
    }
    pub fn busy(&self) -> bool {
        self.phase != Phase::Idle
    }
    fn idle(&mut self) {
        self.phase = Phase::Idle;
        self.pending = false;
        self.header = None;
        self.encoder = None;
        self.lease = false;
    }
    pub fn start(&mut self, args: &[u8], sb: &mut Scrollback) {
        if self.busy() {
            sb.push(b"A capture is already active.");
            return;
        }
        if crate::instance_pd() != c::CLIENT {
            sb.push(b"Capture is available in the first terminal only.");
            return;
        }
        if args.is_empty()
            || !args.ends_with(b".png")
            || args.iter().any(|b| b.is_ascii_whitespace())
        {
            sb.push(b"Usage: capture FILE.png (whole desktop, native size)");
            return;
        }
        if unsafe {
            crate::fs_commands::FS.busy()
                || crate::fetch_command::FETCH.busy()
                || crate::update_command::UPDATE.busy()
        } {
            sb.push(b"Finish the current file operation before capture.");
            return;
        }
        let Some(n) = (unsafe { crate::fs_commands::FS.join(args, &mut self.path) }) else {
            sb.push(b"Capture path is too long.");
            return;
        };
        if n > f::PATH_BYTES {
            sb.push(b"Capture path is too long.");
            return;
        }
        let Ok(bank) = sex_pdx::screen_capture_bank(false) else {
            sb.push(b"This terminal has no capture grant.");
            return;
        };
        let epoch = bank.service_epoch();
        if epoch == 0 {
            sb.push(b"Display capture is not ready; try again.");
            return;
        }
        let Some(sequence) = self.sequence.checked_add(1) else {
            sb.push(b"Capture sequence exhausted; reopen the terminal.");
            return;
        };
        self.bank = Some(bank);
        self.epoch = epoch;
        self.sequence = sequence;
        self.task = crate::INSTANCE_TASK.load(core::sync::atomic::Ordering::Acquire);
        self.path_len = n;
        self.started = sex_pdx::get_ticks();
        self.phase = Phase::Announce;
        self.pending = false;
        self.to_files = false;
        self.offset = 0;
        self.chunk = 0;
        self.lease = false;
        self.revoke_sent = false;
        self.cancelled = false;
        self.saved = false;
        sb.push(b"Capturing the whole desktop.");
        sb.push(b"Native resolution. Escape cancels.");
        sex_pdx::serial_record!("[spindle.capture.intent] target={} task={} sequence={} epoch={} path_prefix={} path_bytes={} whole_desktop=1 native=1",c::CLIENT,self.task,sequence,self.epoch,core::str::from_utf8(&self.path[..n.min(72)]).unwrap_or("?"),n);
    }
    fn fail(&mut self, sb: &mut Scrollback, why: &[u8]) {
        self.cancelled = true;
        sb.push(why);
        self.header = None;
        self.encoder = None;
        if self.pending && self.to_files {
            self.phase = Phase::Fenced;
            sb.push(b"Waiting for Files before releasing its request buffer.");
        } else if self.lease {
            self.phase = Phase::Release;
            self.pending = false;
        } else {
            self.idle();
        }
    }
    pub fn cancel(&mut self, sb: &mut Scrollback) {
        if self.busy() {
            self.fail(sb, b"Capture cancelled. A partial PNG may remain.");
        }
    }
    fn file_region() -> Option<&'static mut [u8]> {
        use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
        let v = sex_pdx::memory_region_view(
            f::CLIENT_SLOT,
            0,
            f::REGION_BYTES as u64,
            QUERY_VIEW | READ | WRITE,
        )
        .ok()?;
        Some(unsafe {
            core::slice::from_raw_parts_mut(v.virtual_address as *mut u8, f::REGION_BYTES)
        })
    }
    fn display(&mut self, kind: u64) -> Result<(), u64> {
        sex_pdx::pdx_call_checked(
            sex_pdx::SLOT_DISPLAY,
            c::OP,
            kind,
            self.sequence as u64,
            self.epoch,
        )?;
        self.pending = true;
        self.to_files = false;
        self.sent_at = sex_pdx::get_ticks();
        Ok(())
    }
    fn file(&mut self, kind: u8, length: u64) -> Result<(), u64> {
        let region = Self::file_region().ok_or(sex_pdx::ERR_CAP_INVALID)?;
        region[..self.path_len].copy_from_slice(&self.path[..self.path_len]);
        let n = self.path_len;
        let sequence = (self.file_seq + 1) & 0xff_ffff;
        let r = f::Request {
            seq: sequence,
            kind,
            path_len: n as u16,
            offset: if kind == f::WRITE { self.offset } else { 0 },
            length,
        };
        let a = r.args();
        sex_pdx::pdx_call_checked(sex_pdx::SLOT_STORAGE, f::OP, a[0], a[1], a[2])?;
        self.file_seq = sequence;
        self.pending = true;
        self.to_files = true;
        self.sent_at = sex_pdx::get_ticks();
        Ok(())
    }
    pub fn step(&mut self, sb: &mut Scrollback, display_idle: bool) -> bool {
        if !self.busy() {
            return false;
        }
        let now = sex_pdx::get_ticks();
        if matches!(
            self.phase,
            Phase::Stat | Phase::Create | Phase::Write | Phase::Sync
        ) {
            if self
                .header
                .is_some_and(|h| self.bank.as_ref().and_then(|b| b.header()) != Some(h))
            {
                self.fail(
                    sb,
                    b"Capture lease changed before file handoff; no success is confirmed.",
                );
                return true;
            }
        }
        if self.phase == Phase::Fenced
            && self.pending
            && self.to_files
            && self.lease
            && !self.revoke_sent
        {
            if sex_pdx::pdx_call_checked(
                sex_pdx::SLOT_DISPLAY,
                c::OP,
                c::RELEASE,
                self.sequence as u64,
                self.epoch,
            )
            .is_ok()
            {
                self.revoke_sent = true;
            }
        }
        if self.pending {
            if now.saturating_sub(self.sent_at) > 15_000 && self.phase != Phase::Fenced {
                self.fail(sb, b"Capture timed out; no saved screenshot is confirmed.");
                return true;
            }
            return false;
        }
        let result = match self.phase {
            Phase::Announce => {
                if !display_idle {
                    return false;
                }
                self.display(c::BEGIN).map(|_| {
                    self.lease = true;
                    self.phase = Phase::Frame
                })
            }
            Phase::Stat => self.file(f::STAT, 0),
            Phase::Create => self.file(f::CREATE, 0),
            Phase::Write => {
                if self.chunk == 0 {
                    let (Some(bank), Some(header), Some(encoder)) =
                        (self.bank.as_ref(), self.header, self.encoder.as_mut())
                    else {
                        self.fail(sb, b"Capture lost its frame.");
                        return true;
                    };
                    if bank.header() != Some(header) {
                        self.fail(sb, b"Capture lease expired; partial file was not saved.");
                        return true;
                    }
                    let Some(region) = Self::file_region() else {
                        self.fail(sb, b"Capture has no Files buffer.");
                        return true;
                    };
                    let Ok(n) = encoder.fill_with(
                        c::pixels(header.width, header.height).unwrap(),
                        |i| bank.pixel(i).expect("CAPTURE_PIXEL_BOUNDS"),
                        &mut region[f::PATH_BYTES..f::PATH_BYTES + 32 * 1024],
                    ) else {
                        self.fail(sb, b"PNG encoding failed.");
                        return true;
                    };
                    if bank.header() != Some(header) {
                        self.fail(
                            sb,
                            b"Capture changed during encoding; partial file was not saved.",
                        );
                        return true;
                    }
                    if n == 0 {
                        if self.offset != encoder.total_bytes() as u64 {
                            self.fail(sb, b"PNG stream ended at the wrong length.");
                            return true;
                        }
                        self.phase = Phase::Sync;
                        return false;
                    }
                    self.chunk = n;
                }
                self.file(f::WRITE, self.chunk as u64)
            }
            Phase::Sync => self.file(f::SYNC, 0),
            Phase::Release => self.display(c::RELEASE),
            _ => return false,
        };
        match result {
            Ok(()) => false,
            Err(sex_pdx::ERR_SERVICE_NOT_READY) => false,
            Err(_) => {
                self.fail(sb, b"Capture transport failed; no success is confirmed.");
                true
            }
        }
    }
    pub fn receive(&mut self, caller: u32, word: u64, sb: &mut Scrollback) -> bool {
        if !self.busy() {
            return false;
        }
        if caller == c::DISPLAY {
            if self.cancelled && self.pending && self.to_files {
                if let Some((sequence, status, c::RELEASE)) = c::decode_reply(word) {
                    if sequence == self.sequence {
                        if status == c::OK {
                            self.lease = false;
                        }
                        return true;
                    }
                }
            }
            if let Some((sequence, c::CANCELLED, c::BEGIN)) = c::decode_reply(word) {
                if sequence == self.sequence {
                    self.fail(
                        sb,
                        b"Display cancelled the capture lease; no success is confirmed.",
                    );
                    return true;
                }
            }
        }
        if !self.pending {
            return false;
        }
        if self.to_files {
            if caller != 11 {
                return false;
            }
            let Some((sequence, status, value)) = f::decode_reply(word) else {
                return false;
            };
            if sequence != self.file_seq {
                return false;
            }
            self.pending = false;
            if self.phase == Phase::Fenced {
                if self.lease {
                    self.phase = Phase::Release
                } else {
                    self.idle();
                }
                return true;
            }
            if self.phase == Phase::Stat {
                if status == f::NOT_FOUND {
                    self.phase = Phase::Create;
                    return true;
                }
                self.fail(
                    sb,
                    if status == f::OK {
                        b"Capture destination already exists."
                    } else {
                        b"Capture destination could not be checked."
                    },
                );
                return true;
            }
            if status != f::OK {
                self.fail(sb, b"Capture file write failed; a partial PNG may remain.");
                return true;
            }
            match self.phase {
                Phase::Create => self.phase = Phase::Write,
                Phase::Write => {
                    if value as usize != self.chunk {
                        self.fail(sb, b"Capture stopped after a short file write.");
                        return true;
                    }
                    self.offset += value as u64;
                    self.chunk = 0;
                }
                Phase::Sync => {
                    self.saved = true;
                    self.phase = Phase::Release;
                }
                _ => {}
            }
            true
        } else {
            if caller != c::DISPLAY {
                return false;
            }
            let Some((sequence, status, kind)) = c::decode_reply(word) else {
                return false;
            };
            if sequence != self.sequence {
                return false;
            }
            if (self.phase == Phase::Frame && kind != c::BEGIN)
                || (self.phase == Phase::Release && kind != c::RELEASE)
            {
                return true;
            }
            self.pending = false;
            if self.phase == Phase::Frame {
                if status != c::OK {
                    self.lease = false;
                    self.fail(
                        sb,
                        if status == c::OVERSIZE {
                            b"Native desktop exceeds the 16 MiB capture limit."
                        } else {
                            b"Display refused the capture."
                        },
                    );
                    return true;
                }
                let Some(header) = self
                    .bank
                    .as_ref()
                    .and_then(|b| b.header())
                    .filter(|h| h.epoch == self.epoch && h.valid(self.sequence, self.task))
                else {
                    self.fail(sb, b"Display returned no owned capture frame.");
                    return true;
                };
                let Ok(encoder) = Encoder::new(header.width, header.height) else {
                    self.fail(sb, b"Capture dimensions cannot be encoded.");
                    return true;
                };
                self.header = Some(header);
                self.encoder = Some(encoder);
                self.phase = Phase::Stat;
                true
            } else if self.phase == Phase::Release {
                if status == c::OK
                    && self.saved
                    && !self.cancelled
                    && self
                        .bank
                        .as_ref()
                        .is_some_and(|b| b.service_epoch() == self.epoch)
                {
                    sb.push(b"Screenshot saved as PNG at native desktop resolution.");
                    sex_pdx::serial_record!("[spindle.capture.saved] task={} sequence={} epoch={} bytes={} path_prefix={} path_bytes={} synced=1 released=1",self.task,self.sequence,self.epoch,self.offset,core::str::from_utf8(&self.path[..self.path_len.min(72)]).unwrap_or("?"),self.path_len);
                } else {
                    sb.push(b"Capture ended without confirmed success; inspect the partial PNG.");
                }
                self.idle();
                true
            } else {
                true
            }
        }
    }
}
