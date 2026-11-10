// SPDX-License-Identifier: GPL-3.0-only
//! Production Files policy/turn/journal composed with the production nested disk
//! wait and defer queue. Kernel snapshots, filesystem and disk are finite doubles.
#![allow(dead_code, static_mut_refs)]
extern crate self as sex_pdx;
use std::{collections::VecDeque, sync::Mutex};
#[path = "../crates/sex-pdx/src/app_catalog.rs"]
pub mod app_catalog;
#[path = "../crates/sex-pdx/src/crash_log.rs"]
mod crash_wire;
#[path = "../crates/sex-pdx/src/desktop_recording.rs"]
pub mod desktop_recording;
#[path="../crates/sex-pdx/src/document.rs"]pub mod document;
#[path = "../crates/sex-pdx/src/fs.rs"]
pub mod fs;
#[path = "../crates/sex-pdx/src/app_lifecycle.rs"]
mod lifecycle_wire;
#[path = "../crates/sex-pdx/src/program_store.rs"]
mod store_wire;
pub mod program_store {
    pub use super::store_wire::*;
    pub fn load_unbound(name: [u64; 2]) -> u64 {
        crate::unbound_backend::load_unbound(name)
    }
    pub fn load(_name: [u64; 2]) -> u64 {
        crate::nested_io();
        crate::WORLD
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .ordinary_loads += 1;
        loaded(8192)
    }
}
#[path = "../servers/sexfiles/src/defer_queue.rs"]
mod defer_queue;
#[path = "../servers/sexfiles/src/pdx.rs"]
pub mod pdx;
#[path = "../servers/sexfiles/src/recording_image.rs"]
mod recording_image;
#[path = "../servers/sexfiles/src/recording_journal.rs"]
mod recording_journal;
#[path = "../servers/sexfiles/src/recording_role.rs"]
mod recording_role;
#[path = "../servers/sexfiles/src/recording_runtime.rs"]
mod recording_runtime;
pub const SEXFS_V0_META_END_LBA: u64 = 47;
pub const SEXDRIVE_AP4_MULTI_BASE_LBA: u64 = 128;
pub const BLOCK_ERR_BAD_CMD: u64 = 1;
pub const BLOCK_ERR_BAD_LEN: u64 = 2;
pub const BLOCK_ERR_NO_DEVICE: u64 = 3;
pub const SLOT_BLOCK: u64 = 1;
#[macro_export]
macro_rules! serial_println{($($arg:tt)*)=>{{let _=format_args!($($arg)*);}};}
#[macro_export]
macro_rules! serial_record{($($arg:tt)*)=>{{let _=format_args!($($arg)*);}};}
#[derive(Clone, Copy)]
pub struct Message {
    pub type_id: u64,
    pub caller_pd: u32,
    pub caller_task: u32,
    pub arg0: u64,
    pub arg1: u64,
    pub arg2: u64,
}
struct World {
    files_epoch: u64,
    shell_epoch: u64,
    shell_life: crash_wire::PeerLife,
    peer_error: Option<u64>,
    task: u32,
    target_live: bool,
    target_wait: bool,
    keep_allowed: bool,
    keeps: usize,
    capacity: usize,
    sent: Vec<(u32, u64)>,
    requests: VecDeque<Message>,
    nested: Vec<Message>,
    io_count: usize,
    prepare_count: usize,
    ordinary_loads: usize,
    bindings: Vec<Option<u32>>,
    data_calls: usize,
    on_io: Option<fn()>,
    on_send: Option<fn()>,
    root_status: desktop_recording::BindStatus,
}
static WORLD: Mutex<Option<World>> = Mutex::new(None);
pub mod app_lifecycle {
    pub use super::lifecycle_wire::*;
    pub fn catalog(_slot: usize) -> Result<CatalogInfo, u64> {
        let w = crate::WORLD.lock().unwrap();
        let w = w.as_ref().unwrap();
        if w.target_wait {
            return Err(RETRY);
        }
        let row = crate::app_catalog::by_target(42).unwrap();
        Ok(CatalogInfo {
            target_pd: 42,
            task_id: w.task,
            surface_id: row.surface,
            coordinator_pd: 3,
        })
    }
    pub fn info(_slot: usize) -> Result<LifecycleInfo, u64> {
        let w = crate::WORLD.lock().unwrap();
        let w = w.as_ref().unwrap();
        if w.target_wait {
            return Err(RETRY);
        }
        if w.target_live {
            Err(LIVE)
        } else {
            Ok(LifecycleInfo {
                target_pd: 42,
                task_id: w.task,
                required_mask: 31,
                acked_mask: 31,
            })
        }
    }
}
pub mod crash_log {
    pub use super::crash_wire::*;
    pub fn restarts() -> u64 {
        crate::WORLD.lock().unwrap().as_ref().unwrap().files_epoch - 1
    }
    pub fn peer_state(slot: u64) -> Result<PeerState, u64> {
        assert_eq!(slot, 6);
        let w = crate::WORLD.lock().unwrap();
        let w = w.as_ref().unwrap();
        if let Some(error) = w.peer_error {
            return Err(error);
        }
        PeerState::decode(w.shell_epoch, w.shell_life as u64)
    }
    pub fn keep(address: u64, pages: u64) -> bool {
        assert_eq!(address % 4096, 0);
        assert_eq!(pages, 1);
        let mut w = crate::WORLD.lock().unwrap();
        let w = w.as_mut().unwrap();
        w.keeps += 1;
        w.keep_allowed
    }
}
pub fn pdx_reply(caller: u32, word: u64) -> u64 {
    if let Some(reply) = desktop_recording::ImageReply::decode(desktop_recording::FILES, word) {
        if reply.status == desktop_recording::BindStatus::Ok {
            let expected = if reply.kind == desktop_recording::ImageKind::Read {
                recording_journal::Fence::Held(image_request(1))
            } else {
                recording_journal::Fence::Releasing(image_request(1))
            };
            assert_eq!(
                recording_runtime::journal_for_test(),
                Ok(expected),
                "checkpoint before positive/cleanup send"
            );
        }
    }
    let callback = {
        let mut w = WORLD.lock().unwrap();
        let w = w.as_mut().unwrap();
        if w.capacity == 0 {
            return 6;
        }
        w.capacity -= 1;
        w.sent.push((caller, word));
        w.on_send.take()
    };
    if let Some(callback) = callback {
        callback();
    }
    0
}
pub fn pdx_call(_slot: u64, _op: u64, _a0: u64, _a1: u64, _a2: u64) -> (u64, u64) {
    (0, 0)
}
pub fn pdx_listen_raw(_slot: u64) -> Message {
    WORLD
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .requests
        .pop_front()
        .expect("finite nested reply stream exhausted")
}
pub fn sys_yield() {}
mod nested_disk {
    use super::{pdx_call, pdx_listen_raw, serial_println, sys_yield, SLOT_BLOCK};
    const BLOCK_READ: u64 = 1;
    const BLOCK_WRITE: u64 = 2;
    const BLOCK_SECTOR_SIZE: u64 = 512;
    fn hot_log() -> bool {
        false
    }
    pub struct DiskFs;
    include!(env!("RECORDING_NESTED_DISK_SOURCE"));
}
fn nested_io() {
    let callback = {
        let mut w = WORLD.lock().unwrap();
        let w = w.as_mut().unwrap();
        assert_eq!(w.keeps, 1, "KEEP before any image admission");
        w.io_count += 1;
        let messages = std::mem::take(&mut w.nested);
        w.requests.extend(messages);
        w.requests.push_back(Message {
            type_id: 1,
            caller_pd: 2,
            caller_task: 2,
            arg0: 0,
            arg1: 0,
            arg2: 0,
        });
        w.on_io.take()
    };
    if let Some(callback) = callback {
        callback();
    }
    assert_eq!(nested_disk::DiskFs::diskfs_block_call(1, 0, 512, 7), 0);
}
pub mod fs_service {
    pub fn reply_later(caller: u32, word: u64) {
        let _ = crate::pdx_reply(caller, word);
    }
    pub fn install_recording_binding(task: Option<u32>) {
        crate::WORLD
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .bindings
            .push(task);
    }
    pub fn prepare_recording_root() -> crate::desktop_recording::BindStatus {
        crate::nested_io();
        let mut w = crate::WORLD.lock().unwrap();
        let w = w.as_mut().unwrap();
        w.prepare_count += 1;
        w.root_status
    }
    pub fn handle(caller: u32, a: u64, _offset: u64, _length: u64) -> u64 {
        assert_eq!(caller, 42);
        crate::nested_io();
        crate::WORLD.lock().unwrap().as_mut().unwrap().data_calls += 1;
        crate::fs::reply((a & 0xffffff) as u32, crate::fs::OK, 32)
    }
}
// The loader boundary is a finite backend; the SDK packing functions are actual.
mod unbound_backend {
    pub fn load_unbound(name: [u64; 2]) -> u64 {
        assert_eq!(name, crate::program_store::pack_name(b"record").unwrap());
        crate::nested_io();
        crate::program_store::loaded(4096)
    }
}
include!(env!("RECORDING_RUNTIME_CASES"));
