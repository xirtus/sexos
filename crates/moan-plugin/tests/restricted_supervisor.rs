//! Actual parent adapter with finite Files, lifecycle and inbox backends.
extern crate alloc;
extern crate self as sex_app;
extern crate self as sex_pdx;
#[macro_export] macro_rules! serial_record {($($t:tt)*)=>{{let _=format_args!($($t)*);}}}
pub use moan_plugin::job;
#[path = "../../sex-pdx/src/restricted_helper.rs"]
pub mod restricted_helper;
#[path = "../../sex-pdx/src/restricted_job.rs"]
pub mod restricted_job;
#[path = "../src/supervisor.rs"]
mod supervisor;
use restricted_helper::Subject;
use restricted_job::Info;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Mutex,
};
static SERIAL: Mutex<()> = Mutex::new(());
static WORLD: Mutex<Option<World>> = Mutex::new(None);
pub const SLOT_SHELL: u64 = 6;
struct World {
    now: u64,
    files_epoch: u64,
    shell_epoch: u64,
    disk: BTreeMap<Vec<u8>, Vec<u8>>,
    info: Option<Info>,
    events: VecDeque<launch::Event>,
    observations: VecDeque<restricted_job::Observation>,
    calls: Vec<(u64, [u64; 3])>,
    released: usize,
    tamper_prepare: bool,
    cleanup_full: bool,
    cleanup_error: bool,
}
fn with<R>(f: impl FnOnce(&mut World) -> R) -> R {
    f(WORLD.lock().unwrap().as_mut().unwrap())
}
fn reset() {
    *WORLD.lock().unwrap() = Some(World {
        now: 0,
        files_epoch: 1,
        shell_epoch: 1,
        disk: BTreeMap::new(),
        info: None,
        events: VecDeque::new(),
        observations: VecDeque::new(),
        calls: Vec::new(),
        released: 0,
        tamper_prepare: false,
        cleanup_full: false,
        cleanup_error: false,
    });
}
pub fn get_ticks() -> u64 {
    with(|w| w.now)
}
pub fn pdx_call(_: u64, op: u64, a: u64, b: u64, c: u64) -> (u64, u64) {
    with(|w| w.calls.push((op, [a, b, c])));
    (0, 0)
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
        Ok(Peer {
            epoch: crate::with(|w| w.shell_epoch),
        })
    }
}
pub mod app_catalog {
    #[derive(PartialEq)]
    pub enum App {
        Program,
    }
    pub struct Row {
        pub app: App,
        pub target: u32,
        pub surface: u64,
    }
    pub fn by_target(pd: u32) -> Option<Row> {
        (pd == 42).then_some(Row {
            app: App::Program,
            target: 42,
            surface: 360,
        })
    }
}
pub mod app_lifecycle {
    pub const SELF_CATALOG_SLOT: usize = 48;
    pub struct Catalog {
        pub target_pd: u32,
        pub task_id: u32,
    }
    impl Catalog {
        pub fn is_subject(&self, p: u32, s: u64, c: u32) -> bool {
            (p, s, c) == (42, 360, 3)
        }
    }
    pub fn catalog(_: usize) -> Result<Catalog, ()> {
        Ok(Catalog {
            target_pd: 42,
            task_id: 99,
        })
    }
}
pub mod launch {
    #[derive(Clone, Copy)]
    pub enum Event {
        Started {
            cookie: u32,
            row: u32,
            task: u32,
        },
        Done {
            cookie: u32,
            row: u32,
            task: u32,
            status: Option<u32>,
        },
        Refused {
            cookie: u32,
            code: u32,
        },
    }
    pub fn poll(_: u32) -> Option<Event> {
        crate::with(|w| w.events.pop_front())
    }
}
pub mod restricted_status {
    pub fn query(_: u32, _: u32, _: Option<crate::Subject>, _: u64) -> bool {
        true
    }
    pub fn poll(
        cookie: u32,
        ticket: u32,
        epoch: u64,
    ) -> Option<crate::restricted_job::Observation> {
        crate::with(|w| {
            let i = w
                .observations
                .iter()
                .position(|o| o.cookie == cookie && o.ticket == ticket && o.shell_epoch == epoch)?;
            w.observations.remove(i)
        })
    }
}
pub mod files {
    pub mod direct {
        pub struct Stat {
            pub dir: bool,
            pub size: u64,
        }
        pub fn stat(path: &[u8]) -> Result<Stat, u8> {
            crate::with(|w| {
                w.disk
                    .get(path)
                    .map(|b| Stat {
                        dir: false,
                        size: b.len() as u64,
                    })
                    .ok_or(1)
            })
        }
        pub fn create(path: &[u8]) -> Result<(), u8> {
            crate::with(|w| {
                if w.disk.contains_key(path) {
                    return Err(2);
                }
                w.disk.insert(path.to_vec(), Vec::new());
                Ok(())
            })
        }
        pub fn write_at(path: &[u8], at: u64, bytes: &[u8]) -> Result<usize, u8> {
            crate::with(|w| {
                let b = w.disk.get_mut(path).ok_or(1)?;
                b.resize(at as usize + bytes.len(), 0);
                b[at as usize..].copy_from_slice(bytes);
                Ok(bytes.len())
            })
        }
        pub fn sync() -> Result<(), u8> {
            Ok(())
        }
        pub fn read_at(path: &[u8], at: u64, out: &mut [u8]) -> Result<usize, u8> {
            crate::with(|w| {
                let b = w.disk.get(path).ok_or(1)?;
                let at = at as usize;
                if at > b.len() {
                    return Err(3);
                }
                let n = out.len().min(b.len() - at);
                out[..n].copy_from_slice(&b[at..at + n]);
                Ok(n)
            })
        }
        pub mod restricted {
            pub fn stat(path: &[u8], _: u64) -> Result<(bool, u64), u8> {
                super::stat(path).map(|s| (s.dir, s.size))
            }
            pub fn create(path: &[u8], _: u64) -> Result<(), u8> {
                super::create(path)
            }
            pub fn sync(_: u64) -> Result<(), u8> {
                super::sync()
            }
            pub fn write_at(path: &[u8], at: u64, bytes: &[u8], _: u64) -> Result<usize, u8> {
                super::write_at(path, at, bytes)
            }
            pub fn read_at(path: &[u8], at: u64, out: &mut [u8], _: u64) -> Result<usize, u8> {
                super::read_at(path, at, out)
            }
            pub struct End {
                done: bool,
            }
            impl End {
                pub fn poll(&mut self) -> Option<Result<(), u8>> {
                    if self.done || crate::with(|w| w.cleanup_full) {
                        return None;
                    }
                    if crate::with(|w| w.cleanup_error) {
                        self.done = true;
                        crate::with(|w| w.cleanup_full = true);
                        return Some(Err(1));
                    }
                    self.done = true;
                    crate::with(|w| w.released += 1);
                    Some(Ok(()))
                }
            }
            pub fn begin_end(_: u32, _: [u8; 16], _: u64, _: bool) -> Result<Option<End>, u8> {
                Ok(Some(End { done: false }))
            }
            pub fn epoch() -> Result<u64, u8> {
                Ok(crate::with(|w| w.files_epoch))
            }
            pub fn prepare(p: &crate::restricted_job::Prepare<'_>) -> Result<crate::Info, u8> {
                crate::with(|w| {
                    let bytes = w.disk.get(p.paths[1]).ok_or(1)?;
                    let mut info = crate::Info {
                        ticket: 7,
                        frames: u32::from_le_bytes(bytes[28..32].try_into().unwrap()),
                        files_epoch: w.files_epoch,
                        nonce: p.nonce,
                        plugin_hash: [2; 32],
                        input_hash: <[u8; 32]>::from(<crate::Sha256 as crate::Digest>::digest(
                            bytes,
                        )),
                        output_hash: [0; 32],
                        parent: crate::Subject { pd: 42, task: 99 },
                        child: None,
                        generation: 0,
                        format: p.format as u32,
                        cookie: p.cookie,
                        coordinator_epoch: 0,
                    };
                    if w.tamper_prepare {
                        info.input_hash[0] ^= 1;
                    }
                    w.info = Some(info);
                    Ok(info)
                })
            }
            pub fn result(_: u32, _: [u8; 16], _: u64) -> Result<crate::Info, u8> {
                crate::with(|w| w.info.ok_or(1))
            }
            pub fn release(_: u32, _: [u8; 16], _: u64) -> Result<(), u8> {
                crate::with(|w| w.released += 1);
                Ok(())
            }
            pub fn cancel(_: u32, _: [u8; 16], _: u64) -> Result<(), u8> {
                crate::with(|w| w.released += 1);
                Ok(())
            }
        }
    }
}
fn start() -> supervisor::OfflineJob {
    supervisor::OfflineJob::start_with(
        b"@/plugin",
        b"@/request",
        b"@/result",
        [7; 16],
        9,
        &[[0.25, -0.5]; 3],
        &[],
        job::Format::Clap,
        0,
    )
    .unwrap()
}
fn complete(code: Option<u32>) {
    with(|w| {
        w.events.push_back(launch::Event::Started {
            cookie: 9,
            row: 43,
            task: 101,
        });
        w.events.push_back(launch::Event::Done {
            cookie: 9,
            row: 43,
            task: 101,
            status: code,
        });
        let bytes = job::encode([7; 16], &[[0.5, -0.25]; 3]).unwrap();
        w.disk.insert(b"@/result".to_vec(), bytes.clone());
        let info = w.info.as_mut().unwrap();
        info.child = Some(Subject { pd: 43, task: 101 });
        info.generation = 1;
        info.coordinator_epoch = 1;
        info.output_hash = Sha256::digest(&bytes).into();
    });
}
#[test]
fn immutable_metadata_and_digest_precede_single_publication() {
    let _serial = SERIAL.lock().unwrap();
    reset();
    let mut j = start();
    assert!(with(|w| w
        .calls
        .iter()
        .any(|(op, _)| *op == restricted_job::OP_START)));
    assert!(!j.retired());
    assert!(j.read_result().unwrap().is_none());
    complete(Some(0));
    j.poll(20);
    assert!(j.retired());
    assert_eq!(
        j.child_identity(),
        Some(job::Identity { row: 43, task: 101 })
    );
    assert_eq!(j.read_result().unwrap().unwrap(), [[0.5, -0.25]; 3]);
    with(|w| w.info.as_mut().unwrap().input_hash[0] ^= 1);
    assert!(j.read_result().is_err());
    with(|w| w.info.as_mut().unwrap().input_hash[0] ^= 1);
    with(|w| w.disk.get_mut(b"@/result".as_slice()).unwrap()[40] ^= 1);
    assert!(j.read_result().is_err());
    with(|w| w.disk.get_mut(b"@/result".as_slice()).unwrap()[40] ^= 1);
    assert!(j.take_result().unwrap().is_some());
    assert!(j.take_result().unwrap().is_none());
    assert!(!j.retired());
    j.poll(40);
    assert!(j.retired());
    assert_eq!(with(|w| w.released), 1);
}
#[test]
fn cleanup_backpressure_keeps_retirement_pending_without_blocking_poll() {
    let _serial = SERIAL.lock().unwrap();
    reset();
    let mut j = start();
    complete(Some(0));
    j.poll(20);
    assert!(j.take_result().unwrap().is_some());
    with(|w| w.cleanup_full = true);
    for now in 21..100 {
        j.poll(now);
        assert!(!j.retired());
    }
    assert_eq!(with(|w| w.released), 0);
    with(|w| w.cleanup_full = false);
    j.poll(100);
    assert!(j.retired());
    assert_eq!(with(|w| w.released), 1);
}
#[test]
fn mismatched_initial_snapshot_never_enqueues_start() {
    let _serial = SERIAL.lock().unwrap();
    reset();
    with(|w| w.tamper_prepare = true);
    let result = supervisor::OfflineJob::start_with(
        b"@/plugin",
        b"@/request",
        b"@/result",
        [7; 16],
        9,
        &[[0.25, -0.5]; 3],
        &[],
        job::Format::Clap,
        0,
    );
    assert!(result.is_err());
    assert!(with(|w| w.calls.is_empty()));
}
#[test]
fn crash_cancel_and_late_refusal_never_publish_or_invent_retirement() {
    let _serial = SERIAL.lock().unwrap();
    reset();
    let mut j = start();
    with(|w| {
        w.events.push_back(launch::Event::Started {
            cookie: 9,
            row: 43,
            task: 101,
        })
    });
    j.poll(20);
    with(|w| {
        w.events
            .push_back(launch::Event::Refused { cookie: 9, code: 1 })
    });
    j.poll(40);
    assert!(!j.retired());
    j.cancel();
    j.poll(300);
    assert!(!j.retired());
    with(|w| {
        w.events.push_back(launch::Event::Done {
            cookie: 9,
            row: 43,
            task: 102,
            status: Some(0),
        })
    });
    j.poll(320);
    assert!(!j.retired());
    with(|w| {
        w.events.push_back(launch::Event::Done {
            cookie: 9,
            row: 43,
            task: 101,
            status: None,
        })
    });
    j.poll(340);
    assert!(j.retired());
    assert_eq!(j.status(), job::Status::Failed);
    assert!(j.read_result().unwrap().is_none());
}
#[test]
fn restarted_shell_requires_known_exact_retirement_observation() {
    let _serial = SERIAL.lock().unwrap();
    reset();
    let mut j = start();
    with(|w| {
        w.events.push_back(launch::Event::Started {
            cookie: 9,
            row: 43,
            task: 101,
        })
    });
    j.poll(20);
    with(|w| w.shell_epoch = 2);
    j.poll(300);
    assert!(!j.retired());
    let observation = restricted_job::Observation {
        cookie: 9,
        ticket: 7,
        state: restricted_job::STATUS_UNKNOWN,
        recovery_complete: true,
        current: None,
        shell_epoch: 2,
    };
    with(|w| w.observations.push_back(observation));
    j.poll(600);
    assert!(!j.retired());
    with(|w| {
        w.observations.push_back(restricted_job::Observation {
            state: restricted_job::STATUS_RETIRED,
            current: Some(Subject { pd: 43, task: 102 }),
            ..observation
        })
    });
    j.poll(900);
    assert!(j.retired());
    assert_eq!(j.status(), job::Status::Failed);
    assert!(j.read_result().unwrap().is_none());
}
#[test]
fn cleanup_error_does_not_complete_until_verified_files_replacement() {
    let _serial = SERIAL.lock().unwrap();
    reset();
    let mut j = start();
    complete(Some(0));
    j.poll(20);
    assert!(j.take_result().unwrap().is_some());
    with(|w| w.cleanup_error = true);
    j.poll(40);
    assert!(!j.retired());
    j.poll(60);
    assert!(!j.retired());
    with(|w| w.files_epoch = 2);
    j.poll(80);
    assert!(j.retired());
}
#[test]
fn recovery_complete_without_started_is_not_retirement() {
    let _serial = SERIAL.lock().unwrap();
    reset();
    let mut j = start();
    with(|w| w.shell_epoch = 2);
    with(|w| {
        w.observations.push_back(restricted_job::Observation {
            cookie: 9,
            ticket: 7,
            state: restricted_job::STATUS_UNKNOWN,
            recovery_complete: true,
            current: None,
            shell_epoch: 2,
        })
    });
    j.poll(300);
    assert!(!j.retired());
    assert!(j.child_identity().is_none());
}
