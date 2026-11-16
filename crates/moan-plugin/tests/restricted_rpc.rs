//! Actual SDK private RPC with a finite clock, provider and region backend.
extern crate self as sex_pdx;
#[path = "../../sex-pdx/src/fs.rs"]
pub mod fs;
#[path = "../../sex-pdx/src/restricted_helper.rs"]
pub mod restricted_helper;
#[path = "../../sex-pdx/src/restricted_job.rs"]
pub mod restricted_job;
pub mod app_catalog {
    pub const SPINDLE_TARGETS: [u32; 1] = [41];
}
use std::{
    cell::UnsafeCell,
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
static SERIAL: Mutex<()> = Mutex::new(());
static NEXT: AtomicU64 = AtomicU64::new(1);
static WORLD: Mutex<Option<World>> = Mutex::new(None);
struct Region(UnsafeCell<[u8; fs::REGION_BYTES]>);
unsafe impl Sync for Region {}
static REGION: Region = Region(UnsafeCell::new([0; fs::REGION_BYTES]));
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Delay,
    Unavailable,
    Changed,
    Timeout,
    Full,
}
struct World {
    epoch: u64,
    now: u64,
    mode: Mode,
    admitted: usize,
    attempts: usize,
    request: [u64; 3],
    delivered: bool,
    mail: VecDeque<PdxMessage>,
}
#[derive(Clone, Copy)]
pub struct PdxMessage {
    pub type_id: u64,
    pub caller_pd: u32,
    pub caller_task: u32,
    pub arg0: u64,
    pub arg1: u64,
    pub arg2: u64,
}
pub const SLOT_STORAGE: u64 = 7;
fn with<R>(f: impl FnOnce(&mut World) -> R) -> R {
    f(WORLD.lock().unwrap().as_mut().unwrap())
}
fn reset(mode: Mode) -> u64 {
    let epoch = NEXT.fetch_add(2, Ordering::Relaxed);
    *WORLD.lock().unwrap() = Some(World {
        epoch,
        now: 0,
        mode,
        admitted: 0,
        attempts: 0,
        request: [0; 3],
        delivered: false,
        mail: VecDeque::new(),
    });
    files::direct::restricted::region_ready().unwrap();
    epoch
}
pub fn get_ticks() -> u64 {
    with(|w| w.now)
}
pub fn sys_yield() {
    with(|w| w.now += 100);
}
pub fn sys_wait_self_ticks(_: u64) -> Result<(), u64> {
    sys_yield();
    Ok(())
}
pub mod crash_log {
    pub struct Peer {
        pub epoch: u64,
    }
    impl Peer {
        pub fn live(&self) -> bool {
            true
        }
    }
    pub fn peer_state(_: u64) -> Result<Peer, ()> {
        crate::with(|w| {
            if w.admitted > 0 && w.mode == crate::Mode::Unavailable {
                return Err(());
            }
            Ok(Peer {
                epoch: w.epoch + u64::from(w.admitted > 0 && w.mode == crate::Mode::Changed),
            })
        })
    }
}
pub fn pdx_call(_: u64, _: u64, a: u64, b: u64, c: u64) -> (u64, u64) {
    with(|w| {
        w.attempts += 1;
        if w.mode == Mode::Full && w.attempts < 3 {
            return (1, 0);
        }
        w.admitted += 1;
        w.request = [a, b, c];
        (0, 0)
    })
}
pub fn pdx_try_listen_raw(_: u64) -> Option<PdxMessage> {
    with(|w| {
        if w.admitted == 0 || w.delivered || w.now < 2000 || w.mode == Mode::Timeout {
            return None;
        }
        let action = w.request[1] & 255;
        let value = if action == restricted_job::PREPARE {
            7
        } else {
            0
        };
        if action == restricted_job::PREPARE {
            let mut b = [0u8; 176];
            b[..8].copy_from_slice(b"MOANINF2");
            b[8..12].copy_from_slice(&7u32.to_le_bytes());
            b[12..16].copy_from_slice(&1u32.to_le_bytes());
            b[16..24].copy_from_slice(&w.epoch.to_le_bytes());
            b[24..40].fill(7);
            b[136..144].copy_from_slice(
                &restricted_helper::Subject { pd: 42, task: 99 }
                    .word()
                    .to_le_bytes(),
            );
            b[164..168].copy_from_slice(&9u32.to_le_bytes());
            for (i, byte) in b.iter().enumerate() {
                unsafe {
                    core::ptr::write_volatile(
                        (*REGION.0.get()).as_mut_ptr().add(fs::PATH_BYTES + i),
                        *byte,
                    )
                }
            }
        }
        w.delivered = true;
        Some(PdxMessage {
            type_id: 1,
            caller_pd: 11,
            caller_task: 11,
            arg0: fs::reply((w.request[0] & 0xff_ffff) as u32, fs::OK, value),
            arg1: 0,
            arg2: 0,
        })
    })
}
mod files {
    use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    const FILES: u32 = 11;
    static BUSY: AtomicBool = AtomicBool::new(false);
    static SEQ: AtomicU32 = AtomicU32::new(0);
    struct Turn;
    impl Drop for Turn {
        fn drop(&mut self) {
            BUSY.store(false, Ordering::Release)
        }
    }
    fn region() -> Result<&'static mut [u8], u8> {
        direct::restricted::region_ready()?;
        Ok(unsafe { &mut *crate::REGION.0.get() })
    }
    fn mine(m: &crate::PdxMessage, s: u32) -> bool {
        m.type_id == 1
            && m.caller_pd == FILES
            && crate::fs::decode_reply(m.arg0).is_some_and(|r| r.0 == s)
    }
    fn is_answer(m: &crate::PdxMessage) -> bool {
        m.type_id == 1
    }
    fn is_shell(m: &crate::PdxMessage) -> bool {
        m.caller_pd == 3
    }
    mod mailbox {
        pub fn take(f: impl Fn(&crate::PdxMessage) -> bool) -> Option<crate::PdxMessage> {
            crate::with(|w| {
                let i = w.mail.iter().position(f)?;
                w.mail.remove(i)
            })
        }
        pub fn put(m: crate::PdxMessage) {
            crate::with(|w| w.mail.push_back(m));
        }
    }
    pub mod direct {
        pub mod restricted {
            include!("../../sex-app/src/restricted_files.rs");
        }
    }
}
fn prepare() -> Result<restricted_job::Info, u8> {
    files::direct::restricted::prepare(&restricted_job::Prepare {
        format: 0,
        cookie: 9,
        nonce: [7; 16],
        paths: [b"@/plugin", b"@/request", b"@/output"],
    })
}
#[test]
fn admitted_request_is_not_resent_after_delayed_reply() {
    let _serial = SERIAL.lock().unwrap();
    reset(Mode::Delay);
    assert!(prepare().is_ok());
    assert_eq!(with(|w| w.admitted), 1);
    assert!(with(|w| w.now) >= 2000);
}
#[test]
fn queue_full_retries_only_before_admission() {
    let _serial = SERIAL.lock().unwrap();
    reset(Mode::Full);
    assert!(prepare().is_ok());
    assert_eq!(with(|w| (w.attempts, w.admitted)), (3, 1));
}
#[test]
fn unavailable_after_enqueue_poisons_until_verified_new_epoch() {
    let _serial = SERIAL.lock().unwrap();
    let epoch = reset(Mode::Unavailable);
    assert!(prepare().is_err());
    with(|w| w.mode = Mode::Delay);
    assert!(files::direct::restricted::region_ready().is_err());
    with(|w| w.epoch = epoch + 1);
    assert!(files::direct::restricted::region_ready().is_ok());
}
#[test]
fn timeout_poisons_but_verified_provider_replacement_does_not() {
    let _serial = SERIAL.lock().unwrap();
    reset(Mode::Timeout);
    assert!(prepare().is_err());
    assert!(files::direct::restricted::region_ready().is_err());
    reset(Mode::Changed);
    assert!(prepare().is_err());
    assert!(files::direct::restricted::region_ready().is_ok());
}
#[test]
fn cleanup_poll_is_nonblocking_and_holds_turn_until_ack() {
    let _serial = SERIAL.lock().unwrap();
    let epoch = reset(Mode::Delay);
    let mut end = files::direct::restricted::begin_end(7, [7; 16], epoch, true)
        .unwrap()
        .unwrap();
    assert!(end.poll().is_none());
    assert_eq!(with(|w| w.now), 0);
    assert!(
        files::direct::restricted::begin_end(8, [8; 16], epoch, true)
            .unwrap()
            .is_none()
    );
    with(|w| w.now = 2000);
    assert_eq!(end.poll(), Some(Ok(())));
    assert!(
        files::direct::restricted::begin_end(8, [8; 16], epoch, true)
            .unwrap()
            .is_some()
    );
    assert_eq!(with(|w| w.admitted), 1);
}
#[test]
fn dropping_unacknowledged_cleanup_poison_precedes_unlock() {
    let _serial = SERIAL.lock().unwrap();
    let epoch = reset(Mode::Delay);
    let mut end = files::direct::restricted::begin_end(7, [7; 16], epoch, false)
        .unwrap()
        .unwrap();
    assert!(end.poll().is_none());
    drop(end);
    assert!(files::direct::restricted::region_ready().is_err());
    assert!(files::direct::restricted::begin_end(8, [8; 16], epoch, false).is_err());
}
#[test]
fn staging_file_operation_is_epoch_bounded_too() {
    let _serial = SERIAL.lock().unwrap();
    let epoch = reset(Mode::Unavailable);
    assert!(files::direct::restricted::create(b"@/fresh", epoch).is_err());
    with(|w| w.mode = Mode::Delay);
    assert!(files::direct::restricted::region_ready().is_err());
    assert_eq!(with(|w| w.admitted), 1);
}
#[test]
fn result_reads_use_bounded_payload_and_original_epoch() {
    let _serial = SERIAL.lock().unwrap();
    let epoch = reset(Mode::Delay);
    let mut bytes = vec![0; fs::DATA_MAX + 4];
    assert_eq!(
        files::direct::restricted::read_at(b"@/result", 17, &mut bytes, epoch),
        Ok(0)
    );
    assert_eq!(
        with(|w| w.request[1..].to_vec()),
        vec![17, fs::DATA_MAX as u64]
    );
    assert_eq!(with(|w| w.admitted), 1);
}
