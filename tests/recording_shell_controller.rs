//! Production Shell controller with actual wire and explicit held-authority mocks.
extern crate self as sex_pdx;
use std::sync::Mutex;
#[path = "../crates/sex-pdx/src/desktop_recording.rs"]
pub mod desktop_recording;
#[path = "../crates/sex-pdx/src/desktop_recording_capture.rs"]
pub mod desktop_recording_capture;
#[path = "../crates/sex-pdx/src/memory_region.rs"]
pub mod memory_region;
pub use memory_region::MemoryRegionInfo;
#[path = "../crates/sex-pdx/src/desktop_look.rs"]
pub mod desktop_look;
#[derive(Clone, Copy, Debug)]
pub struct PdxMessage {
    pub type_id: u64,
    pub arg0: u64,
    pub arg1: u64,
    pub arg2: u64,
    pub caller_pd: u32,
    pub caller_task: u32,
}
pub const SLOT_DISPLAY: u64 = 5;
pub const ERR_SERVICE_NOT_READY: u64 = 0x17;
pub mod program_store {
    pub const SHELL_ROUTE_SLOT: u64 = 256;
}
pub mod app_catalog {
    pub const SPINDLE_TARGETS: [u32; 2] = [12, 17];
    pub struct Incarnation {
        pub target: u32,
        pub surface: u64,
        pub route_slot: u64,
        pub lifecycle_slot: usize,
        pub activation_slot: Option<usize>,
        pub close_slot: u64,
    }
    impl Incarnation {
        pub fn frame(&self) -> u64 {
            self.surface
        }
    }
    static SPINDLE: Incarnation = Incarnation {
        target: 12,
        surface: 153,
        route_slot: 14,
        lifecycle_slot: 39,
        activation_slot: None,
        close_slot: 42,
    };
    static RECORD: Incarnation = Incarnation {
        target: 42,
        surface: 500,
        route_slot: 300,
        lifecycle_slot: 301,
        activation_slot: Some(302),
        close_slot: 303,
    };
    pub fn by_target(target: u32) -> Option<&'static Incarnation> {
        match target {
            12 => Some(&SPINDLE),
            42 => Some(&RECORD),
            _ => None,
        }
    }
}
pub mod crash_log {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum PeerLife {
        Live,
        Starting,
        Dead,
        GaveUp,
    }
    pub struct PeerState {
        pub epoch: u64,
        pub life: PeerLife,
    }
    pub fn self_epoch() -> Result<u64, u64> {
        super::HOST.lock().unwrap().epoch.ok_or(5)
    }
    pub fn peer_state(slot: u64) -> Result<PeerState, u64> {
        let h = super::HOST.lock().unwrap();
        let epoch = match slot {
            5 => h.display,
            256 => h.files,
            _ => None,
        }
        .ok_or(5u64)?;
        Ok(PeerState {
            epoch,
            life: PeerLife::Live,
        })
    }
}
pub mod app_lifecycle {
    pub const LIVE: u64 = 5;
    pub const RETRY: u64 = 2;
    pub const ACTIVATION_READY: u64 = 1;
    pub const ACTIVATION_ACTIVE: u64 = 2;
    pub const ACTIVATION_REUSABLE: u64 = 4;
    pub const OP_APP_CLOSE: u64 = 0x108;
    pub const CLOSE_CONFIRMED: u64 = 1;
    pub struct Catalog {
        target: u32,
        task: u32,
    }
    impl Catalog {
        pub fn is_subject(&self, target: u32, _surface: u64, _coordinator: u32) -> bool {
            self.target == target
        }
        pub fn is_sender(&self, target: u32, task: u32) -> bool {
            self.target == target && self.task == task
        }
    }
    pub fn catalog(slot: usize) -> Result<Catalog, u64> {
        let h = super::HOST.lock().unwrap();
        Ok(match slot {
            39 => Catalog {
                target: 12,
                task: 12,
            },
            301 => Catalog {
                target: 42,
                task: h.task,
            },
            _ => return Err(1),
        })
    }
    pub fn info(_slot: usize) -> Result<(), u64> {
        Err(LIVE)
    }
    #[derive(Clone, Copy)]
    pub struct ActivationInfo {
        pub successor: u32,
        pub task: u32,
        pub surface: u64,
        pub state: u64,
    }
    pub fn activation(slot: usize, commit: bool) -> Result<ActivationInfo, u64> {
        assert_eq!(slot, 302);
        let mut h = super::HOST.lock().unwrap();
        if commit {
            h.state = 2;
        }
        Ok(ActivationInfo {
            successor: 42,
            task: h.task,
            surface: 500,
            state: h.state,
        })
    }
    pub fn load_program(slot: usize, len: u32) -> Result<ActivationInfo, u64> {
        assert_eq!((slot, len), (302, 4096));
        let mut h = super::HOST.lock().unwrap();
        h.events.push("load");
        h.state = 5;
        Ok(ActivationInfo {
            successor: 42,
            task: h.task,
            surface: 500,
            state: 5,
        })
    }
    pub fn rebirth(slot: usize) -> Result<ActivationInfo, u64> {
        assert_eq!(slot, 302);
        let mut h = super::HOST.lock().unwrap();
        h.task = 1073741830;
        h.state = 2;
        h.events.push("rebirth");
        Ok(ActivationInfo {
            successor: 42,
            task: h.task,
            surface: 500,
            state: 2,
        })
    }
}
struct Host {
    epoch: Option<u64>,
    files: Option<u64>,
    display: Option<u64>,
    task: u32,
    state: u64,
    calls: Vec<(u64, u64, [u64; 3])>,
    events: Vec<&'static str>,
}
static HOST: Mutex<Host> = Mutex::new(Host {
    epoch: Some(2),
    files: Some(3),
    display: Some(4),
    task: 42,
    state: 4,
    calls: Vec::new(),
    events: Vec::new(),
});
static SERIAL: Mutex<()> = Mutex::new(());
pub fn pdx_call_checked(slot: u64, op: u64, a: u64, b: u64, c: u64) -> Result<u64, u64> {
    let mut h = HOST.lock().unwrap();
    h.calls.push((slot, op, [a, b, c]));
    if op == desktop_recording::OP_FILES_IMAGE && (a >> 56) == 2 {
        h.events.push("release")
    }
    Ok(0)
}
#[path = "../servers/silk-shell/src/desktop_recording.rs"]
mod controller;
use desktop_recording as w;
use desktop_recording_capture as cap;
fn reset() {
    *HOST.lock().unwrap() = Host {
        epoch: Some(2),
        files: Some(3),
        display: Some(4),
        task: 42,
        state: 4,
        calls: Vec::new(),
        events: Vec::new(),
    }
}
fn message(pd: u32, task: u32, op: u64, a: [u64; 3]) -> PdxMessage {
    PdxMessage {
        type_id: op,
        arg0: a[0],
        arg1: a[1],
        arg2: a[2],
        caller_pd: pd,
        caller_task: task,
    }
}
fn intent(c: &mut controller::Controller, i: w::Intent) {
    assert!(c.receive(&message(12, 12, w::OP_SPINDLE, i.args()), 1, false));
    c.poll(2);
}
fn begin(c: &mut controller::Controller) {
    intent(
        c,
        w::Intent::Begin {
            sequence: 1,
            length: 5,
            shell_epoch: 2,
        },
    );
    intent(
        c,
        w::Intent::Chunk {
            sequence: 1,
            offset: 0,
            words: [u64::from_le_bytes(*b"x.mkv\0\0\0"), 0],
        },
    );
    intent(c, w::Intent::Commit { sequence: 1 });
    c.poll(2);
}
fn call(op: u64) -> [u64; 3] {
    HOST.lock()
        .unwrap()
        .calls
        .iter()
        .rev()
        .find(|x| x.1 == op)
        .unwrap()
        .2
}
fn files_reply(c: &mut controller::Controller, word: u64) {
    c.receive(&message(11, 11, 1, [word, 0, 0]), 3, false);
    c.poll(4);
}
fn prepared(c: &mut controller::Controller) -> w::AppReport {
    begin(c);
    let read = w::ImageRequest::decode(3, call(w::OP_FILES_IMAGE)).unwrap();
    files_reply(
        c,
        w::ImageReply::new(read, w::BindStatus::Ok, 4096)
            .unwrap()
            .word(),
    );
    c.poll(5);
    let release = w::ImageRequest::decode(3, call(w::OP_FILES_IMAGE)).unwrap();
    assert_eq!(release.sequence, read.sequence);
    assert_eq!(release.kind, w::ImageKind::Release);
    files_reply(
        c,
        w::ImageReply::new(release, w::BindStatus::Ok, 0)
            .unwrap()
            .word(),
    );
    c.poll(6);
    let bind = w::BindRequest::decode(3, call(w::OP_FILES_BIND)).unwrap();
    assert_eq!(bind.task, 1073741830);
    files_reply(c, w::BindReply::new(bind, w::BindStatus::Ok).word());
    c.poll(7);
    c.poll(8);
    c.poll(9);
    w::AppReport {
        session: w::SessionSequence::new(2, 1).unwrap(),
        status: w::AppStatus::Prepared,
        task: 1073741830,
        files_epoch: 3,
        display_epoch: 4,
        spindle_task: 12,
    }
}
#[test]
fn actual_task_binding_and_image_release_precede_committed_arm() {
    let _s = SERIAL.lock().unwrap();
    reset();
    let mut c = controller::Controller::new();
    let report = prepared(&mut c);
    assert_eq!(HOST.lock().unwrap().events, ["load", "release", "rebirth"]);
    assert!(!HOST
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|x| x.1 == cap::OP_CONTROL));
    c.receive(&message(42, 42, w::OP_RECORDER, report.args()), 10, false);
    c.poll(11);
    assert!(!HOST
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|x| x.1 == cap::OP_CONTROL));
    c.receive(
        &message(42, report.task, w::OP_RECORDER, report.args()),
        12,
        false,
    );
    c.poll(13);
    let arm = cap::ControlRequest::decode(3, call(cap::OP_CONTROL)).unwrap();
    assert_eq!(arm.kind, cap::ControlKind::Arm);
    let ack = cap::ControlReply::new(arm.sequence, report.task, cap::ControlStatus::Armed).unwrap();
    c.receive(&message(1, 1, 1, [ack.word().unwrap(), 0, 0]), 14, false);
    c.poll(15);
    c.poll(16);
    assert!(matches!(
        w::AppCommand::decode(3, call(w::OP_RECORDER)),
        Some(w::AppCommand::Active {
            task: 1073741830,
            ..
        })
    ));
}
#[test]
fn stop_overtakes_pending_arm_and_stale_armed_reply_cannot_activate() {
    let _s = SERIAL.lock().unwrap();
    reset();
    let mut c = controller::Controller::new();
    let report = prepared(&mut c);
    c.receive(
        &message(42, report.task, w::OP_RECORDER, report.args()),
        12,
        false,
    );
    c.poll(13);
    let arm = cap::ControlRequest::decode(3, call(cap::OP_CONTROL)).unwrap();
    intent(&mut c, w::Intent::Stop { sequence: 2 });
    c.poll(14);
    let stop = cap::ControlRequest::decode(3, call(cap::OP_CONTROL)).unwrap();
    assert_eq!(stop.kind, cap::ControlKind::Stop);
    assert_ne!(arm.sequence, stop.sequence);
    let ack = cap::ControlReply::new(arm.sequence, report.task, cap::ControlStatus::Armed).unwrap();
    c.receive(&message(1, 1, 1, [ack.word().unwrap(), 0, 0]), 15, false);
    c.poll(16);
    assert!(!HOST
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|x| x.1 == w::OP_RECORDER
            && matches!(
                w::AppCommand::decode(3, x.2),
                Some(w::AppCommand::Active { .. })
            )));
}
#[test]
fn failed_self_epoch_never_becomes_first_boot_authority() {
    let _s = SERIAL.lock().unwrap();
    reset();
    HOST.lock().unwrap().epoch = None;
    let mut c = controller::Controller::new();
    begin(&mut c);
    assert!(!HOST
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|x| x.1 == w::OP_FILES_IMAGE));
}
