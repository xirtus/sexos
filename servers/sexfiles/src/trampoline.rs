use crate::pdx::{pdx_listen_raw, pdx_reply, serial_println};
use crate::messages;
use crate::vfs;


// SERIAL_DIET_V1: shared budget for hot-loop diagnostics. Early boot keeps
// the full trace (first 120 lines); steady-state loops go quiet so
// serial VM-exits stop dominating wall clock. Error and gate-required
// markers are NOT routed through this.
static mut HOT_LOG_BUDGET: u32 = 120;
#[inline(always)]
fn hot_log() -> bool {
    unsafe {
        if HOT_LOG_BUDGET > 0 { HOT_LOG_BUDGET -= 1; true } else { false }
    }
}

#[no_mangle]
pub extern "C" fn trampoline_main() {
    serial_println!("[sexfiles.trampoline.enter] ok=1");
    assert!(crate::proof::run_memory_authority_negative_proof(),
        "SEXFILES_MEMORY_AUTHORITY_NEGATIVE_FAILED");
    // SLOT_STORAGE is statically assigned; no dynamic registration call is used here.
    serial_println!("[sexfiles.trampoline.before_register] slot=1 ok=1");
    // Optional RamFS proof run (compile-time flag).
    // Matches the silk-shell SCENE_SETTINGS_PROTOCOL_PROOF pattern.
    const RAMFS_PROOF_ENABLED: bool =
        option_env!("SEXFILES_RAMFS_PROOF").is_some();
    if RAMFS_PROOF_ENABLED {
        crate::proof::run_all_proofs();
    }
    const DISKFS_OBJECT_TABLE_PROOF_ENABLED: bool =
        option_env!("SEXOS_DISKFS_OBJECT_TABLE_PROOF").is_some();
    if DISKFS_OBJECT_TABLE_PROOF_ENABLED {
        crate::proof::run_diskfs_object_table_proofs();
    }
    const SEXFILES_JOURNAL_PROOF_ENABLED: bool =
        option_env!("SEXOS_SEXFILES_JOURNAL_PROOF").is_some();
    if SEXFILES_JOURNAL_PROOF_ENABLED {
        crate::proof::run_sexfiles_journal_proofs();
    }
    const SEXFILES_REPLAY_PROOF_ENABLED: bool =
        option_env!("SEXOS_SEXFILES_REPLAY_PROOF").is_some();
    if SEXFILES_REPLAY_PROOF_ENABLED {
        crate::proof::run_sexfiles_replay_proofs();
    }
    const SEXFILES_CAP_RECORD_PROOF_ENABLED: bool =
        option_env!("SEXOS_SEXFILES_CAP_RECORD_PROOF").is_some();
    if SEXFILES_CAP_RECORD_PROOF_ENABLED {
        crate::proof::run_sexfiles_cap_record_proofs();
    }
    const LINEN_SEXFILES_METADATA_PROOF_ENABLED: bool =
        option_env!("SEXOS_LINEN_SEXFILES_METADATA_PROOF").is_some();
    if LINEN_SEXFILES_METADATA_PROOF_ENABLED {
        crate::proof::run_linen_sexfiles_metadata_proofs();
    }
    const SEXFILES_FAULT_INJECTION_PROOF_ENABLED: bool =
        option_env!("SEXOS_SEXFILES_FAULT_INJECTION_PROOF").is_some();
    if SEXFILES_FAULT_INJECTION_PROOF_ENABLED {
        crate::proof::run_sexfiles_fault_injection_proofs();
    }
    const SEXFILES_REAL_BLOCK_PROOF_ENABLED: bool =
        option_env!("SEXOS_SEXFILES_REAL_BLOCK_PROOF").is_some();
    if SEXFILES_REAL_BLOCK_PROOF_ENABLED {
        crate::proof::run_sexfiles_real_block_proofs();
    }
    const SEXFILES_REBOOT_PROOF_ENABLED: bool =
        option_env!("SEXOS_SEXFILES_REBOOT_PROOF").is_some();
    if SEXFILES_REBOOT_PROOF_ENABLED {
        crate::proof::run_sexfiles_reboot_proofs();
    }
    const SEXFILES_EXTENT_PROOF_ENABLED: bool =
        option_env!("SEXOS_SEXFILES_EXTENT_PROOF").is_some();
    if SEXFILES_EXTENT_PROOF_ENABLED {
        crate::proof::run_sexfiles_extent_proofs();
    }
    const SEXFILES_CHECKPOINT_PROOF_ENABLED: bool =
        option_env!("SEXOS_SEXFILES_CHECKPOINT_PROOF").is_some();
    if SEXFILES_CHECKPOINT_PROOF_ENABLED {
        crate::proof::run_sexfiles_checkpoint_proofs();
    }
    const DISKFS_100_AP4_WRITE_PROOF_ENABLED: bool =
        cfg!(sexfiles_diskfs100_ap4_write);
    if DISKFS_100_AP4_WRITE_PROOF_ENABLED {
        crate::proof::run_diskfs100_ap4_write_proof();
        // [sexfiles.diskfs100.ap4.write.profile.done] isolated=1
        return;
    }

    const DISKFS_100_AP4_READ_PROOF_ENABLED: bool =
        cfg!(sexfiles_diskfs100_ap4_read);
    const DISKFS_100_AP5_NEG_READ_NO_WRITE_ENABLED: bool =
        cfg!(sexfiles_diskfs100_ap5_neg_read_no_write);
    if DISKFS_100_AP4_READ_PROOF_ENABLED {
        crate::proof::run_diskfs100_ap4_read_proof();
        // [sexfiles.diskfs100.ap4.read.profile.done] isolated=1
        if DISKFS_100_AP5_NEG_READ_NO_WRITE_ENABLED {
            crate::proof::run_diskfs100_ap5_neg_read_no_write();
        }
        return;
    }

    const DISKFS_100_AP5_NEG_MISMATCH_ENABLED: bool =
        cfg!(sexfiles_diskfs100_ap5_neg_mismatch);
    if DISKFS_100_AP5_NEG_MISMATCH_ENABLED {
        crate::proof::run_diskfs100_ap5_neg_mismatch();
        // [sexfiles.diskfs100.ap5.neg.profile.done] case=mismatch
        return;
    }

    const DISKFS_100_AP5_NEG_MISSING_IMAGE_ENABLED: bool =
        cfg!(sexfiles_diskfs100_ap5_neg_missing_image);
    if DISKFS_100_AP5_NEG_MISSING_IMAGE_ENABLED {
        crate::proof::run_diskfs100_ap5_neg_missing_image();
        // [sexfiles.diskfs100.ap5.neg.profile.done] case=missing_image
        return;
    }

    const DISKFS_100_AP5_NEG_FLUSH_SKIP_ENABLED: bool =
        cfg!(sexfiles_diskfs100_ap5_neg_flush_skip);
    if DISKFS_100_AP5_NEG_FLUSH_SKIP_ENABLED {
        crate::proof::run_diskfs100_ap5_neg_flush_skip();
        return;
    }

    const DISKFS_100_AP6_FLUSH_FSYNC_ENABLED: bool =
        cfg!(sexfiles_diskfs100_ap6_flush_fsync);
    if DISKFS_100_AP6_FLUSH_FSYNC_ENABLED {
        crate::proof::run_diskfs100_ap6_flush_fsync();
        return;
    }
    const DISKFS_NEGATIVE_BOUNDS_AUTH_PROOF_ENABLED: bool =
        cfg!(sexfiles_diskfs_negative_bounds_auth_proof);
    if DISKFS_NEGATIVE_BOUNDS_AUTH_PROOF_ENABLED {
        crate::proof::run_diskfs_negative_bounds_auth_proof();
        return;
    }

    const DISKFS_BRIDGE_STRICT_PROOF_ENABLED: bool =
        cfg!(sexfiles_diskfs_bridge_strict_proof);
    if DISKFS_BRIDGE_STRICT_PROOF_ENABLED {
        crate::proof::run_diskfs_bridge_strict_proof_v1();
        return;
    }

    const DISKFS_100_AP2_PROOF_ENABLED: bool =
        cfg!(sexfiles_diskfs100_ap2_proof);
    if DISKFS_100_AP2_PROOF_ENABLED {
        crate::proof::run_diskfs100_ap2_proof();
        // [sexfiles.diskfs100.ap2.profile.done] isolated=1
        return;
    }

    const DISKFS_100_AP3_PROOF_ENABLED: bool =
        cfg!(sexfiles_diskfs100_ap3_proof);
    if DISKFS_100_AP3_PROOF_ENABLED {
        crate::proof::run_diskfs_multi_object_proofs();
        // [sexfiles.diskfs100.ap3.profile.done] isolated=1
        return;
    }

    const DISKFS_MULTI_OBJECT_PROOF_ENABLED: bool = cfg!(sexfiles_diskfs_multi_object_proof);
    const SEXFILES_ROUTE_AUDIT_ONLY: bool = option_env!("SEXFILES_ROUTE_AUDIT_ONLY").is_some();
    if DISKFS_MULTI_OBJECT_PROOF_ENABLED {
        if SEXFILES_ROUTE_AUDIT_ONLY {
            serial_println!("[sexfiles.disk.multi.skip] reason=route_audit");
        } else {
            crate::proof::run_diskfs_multi_object_proofs();
        }
    }

    const SEXOBJECT_VIEW_PROOF_ENABLED: bool =
        option_env!("SEXOS_SEXOBJECT_VIEW_PROOF").is_some();
    if SEXOBJECT_VIEW_PROOF_ENABLED {
        crate::proof::run_sexobject_view_proof();
    }

    const LINEN_DISK_OBJECT_PROOF_ENABLED: bool =
        option_env!("SEXOS_LINEN_DISK_OBJECT_PROOF").is_some();
    if LINEN_DISK_OBJECT_PROOF_ENABLED {
        crate::proof::run_linen_disk_object_proof();
    }

    const SEXFS_V0_SUPERBLOCK_FORMAT_MOUNT_PROOF_ENABLED: bool =
        option_env!("SEXFS_V0_SUPERBLOCK_FORMAT_MOUNT_PROOF").is_some();
    if SEXFS_V0_SUPERBLOCK_FORMAT_MOUNT_PROOF_ENABLED {
        crate::proof::run_sexfs_v0_superblock_format_mount_proofs();
        // [sexfs.v0.superblock_format_mount.profile.done] non_isolated=1
        // Does not return — VFS loop starts after proof.
    }

    const SEXOBJECT_TABLE_PERSIST_PROOF_ENABLED: bool =
        option_env!("SEXOBJECT_TABLE_PERSIST_PROOF").is_some();
    if SEXOBJECT_TABLE_PERSIST_PROOF_ENABLED {
        crate::proof::run_sexobject_table_persist_proofs();
        // [sexobject.table.persist.profile.done] non_isolated=1
    }

    const SEXOBJECT_TABLE_EXTENT_ALLOC_PROOF_ENABLED: bool =
        option_env!("SEXOBJECT_TABLE_EXTENT_ALLOC_PROOF").is_some();
    if SEXOBJECT_TABLE_EXTENT_ALLOC_PROOF_ENABLED {
        crate::proof::run_sexobject_table_extent_alloc_proofs();
        // [sexobject.extent_alloc.profile.done] non_isolated=1
    }

    const SEXOBJECT_EXTENT_WRITE_FULL_BLOCK_PROOF_ENABLED: bool =
        option_env!("SEXOBJECT_EXTENT_WRITE_FULL_BLOCK_PROOF").is_some();
    if SEXOBJECT_EXTENT_WRITE_FULL_BLOCK_PROOF_ENABLED {
        crate::proof::run_sexobject_extent_write_full_block_proofs();
        // [sexobject.full_block.profile.done] non_isolated=1
    }

    const SEXOBJECT_WRITE_READ_PERSIST_PROOF_ENABLED: bool =
        option_env!("SEXOBJECT_WRITE_READ_PERSIST_PROOF").is_some();
    if SEXOBJECT_WRITE_READ_PERSIST_PROOF_ENABLED {
        crate::proof::run_sexobject_write_read_persist_proofs();
        // [sexobject.write_read.profile.done] non_isolated=1
    }

    const SEXOBJECT_MULTI_OBJECT_PROOF_ENABLED: bool =
        option_env!("SEXOBJECT_MULTI_OBJECT_PROOF").is_some();
    if SEXOBJECT_MULTI_OBJECT_PROOF_ENABLED {
        crate::proof::run_sexobject_multi_object_proofs();
        // [sexobject.multi.profile.done] non_isolated=1
    }

    serial_println!("[sexfiles.ready]");
    serial_println!("[sexfiles.init.ready] slot=1 ok=1");
    serial_println!("[sexfiles.trampoline.loop.enter] ok=1");

    let mut app_custody=sex_pdx::app_lifecycle::ServiceCustody::new();
    static mut DOCUMENTS:crate::document::Service=crate::document::Service::new();
    let documents=unsafe{&mut DOCUMENTS};
    let mut probes=crate::spindle_probe::Service::new();
    let mut history=crate::spindle_history::Service::new();
    let mut durable_history=crate::history_disk::Durable::new();
    loop {
        history.poll(pdx_reply);probes.poll(pdx_reply);documents.poll(pdx_reply);
        static mut SEXFILES_TRAMP_LISTEN_BUDGET: u32 = 8;
        unsafe {
            if SEXFILES_TRAMP_LISTEN_BUDGET > 0 {
                SEXFILES_TRAMP_LISTEN_BUDGET -= 1;
                if hot_log() { serial_println!("[sexfiles.trampoline.listen.enter]"); }
            }
        }
        // SEXFILES_DEFER_V1: drain requests stashed by nested reply-wait
        // loops before listening for new ones — they arrived first.
        let (msg_type, msg_caller, msg_a0, msg_a1, msg_a2) =
            if let Some(d) = crate::pdx::defer_pop() {
                d
            } else {
                // Listen on slot 0 (self message ring — all servers use this pattern).
                let m = if history.pending()||probes.pending()||documents.pending(){match sex_pdx::pdx_try_listen_raw(0){Some(m)=>m,None=>{documents.progress(&mut vfs::DocumentBackend);sex_pdx::sys_yield();continue}}}else{pdx_listen_raw(0)};
                (m.type_id, m.caller_pd, m.arg0, m.arg1, m.arg2)
            };
        let msg = crate::pdx::ReplayMsg {
            type_id: msg_type, caller_pd: msg_caller,
            arg0: msg_a0, arg1: msg_a1, arg2: msg_a2,
        };
        let caller = msg.caller_pd;
        if msg.type_id==sex_pdx::app_lifecycle::OP_APP_EXIT {
            if let Some((notice,acked))=app_custody.receive(&mut sex_pdx::app_lifecycle::KernelAuthority,
                msg.type_id,msg.caller_pd as u64,[msg.arg0,msg.arg1,msg.arg2],|notice| {
                    history.retire(notice.target_pd);probes.retire(notice.target_pd);
                    assert!(documents.retire(notice.target_pd,&mut vfs::DocumentBackend),"DOCUMENT_SNAPSHOT_RETIRE_SCRUB");
                    let (closed,revoked)=vfs::retire_app_subject(notice.target_pd);
                    sex_pdx::serial_record!("[files.app.cleanup] target={} task={} handles={} caps={} saved_data=retained selection=0",
                        notice.target_pd,notice.task_id,closed,revoked);
                }) {
                sex_pdx::serial_record!("[files.app.cleanup.ack] target={} task={} ok={}",notice.target_pd,notice.task_id,acked as u8);
            }
            continue;
        }
        if app_custody.is_retired(caller) { continue; }

        if hot_log() { serial_println!(
            "[sexfiles.trampoline.after_listen] type={:#x} caller={} a0={:#x} a1={:#x}",
            msg.type_id, caller, msg.arg0, msg.arg1
        ); }
        if hot_log() { serial_println!(
            "[sexfiles.route.recv] type={:#x} caller={} a0={:#x} a1={:#x}",
            msg.type_id, caller, msg.arg0, msg.arg1
        ); }

        if msg.type_id==sex_pdx::document::OP{
            if let Some(r)=sex_pdx::document::Request::decode(msg.arg0,msg.arg1,msg.arg2){
                let live=document_caller_live(caller);
                if let Some(v)=documents.handle(caller,live,r,&mut vfs::DocumentBackend){pdx_reply(caller,v);}
            }continue;
        }
        if msg.type_id==sex_pdx::spindle_probe::OP{
            if let Some(r)=sex_pdx::spindle_probe::Request::decode(msg.arg0,msg.arg1,msg.arg2){
                if let Some(v)=probes.handle(caller,crate::spindle_history::live(caller),r,&mut vfs::SpindleProbeBackend){pdx_reply(caller,v);}
            }continue;
        }
        if msg.type_id==sex_pdx::spindle_history::OP {
            if let Some(value)=history.handle(&vfs::RAMFS,caller,crate::spindle_history::live(caller),msg.arg0,msg.arg1,msg.arg2,&mut durable_history){pdx_reply(caller,value);}
            continue;
        }

        // Route message type_id to VFS handler, passing caller PD for namespace/cap check
        let reply = vfs::handle_vfs_message(msg.type_id, msg.arg0, msg.arg1, msg.arg2, caller);

        // If type_id was 0 (empty/spurious), skip reply
        if msg.type_id != 0 {
            if msg.type_id == messages::OP_DISKFS_READ {
                serial_println!(
                    "[sexfiles.bridge.diskfs.read.reply.sent] caller={} value={:#x}",
                    caller, reply
                );
            }
            pdx_reply(caller, reply);
        }
    }
}

/// The message authenticates a PD. Only its immutable kernel lifecycle grant
/// supplies the task identity; a document request carries no replacement task.
fn document_caller_live(caller:u32)->bool {
    let(slot,surface)=match caller{9=>(38,156),16=>(45,301),18=>(54,307),15=>(40,300),_=>return false};
    let live=matches!(sex_pdx::app_lifecycle::catalog(slot),Ok(c) if c.is_subject(caller,surface,3)&&(caller!=15||c.task_id==15))
        && sex_pdx::app_lifecycle::info(slot)==Err(sex_pdx::app_lifecycle::LIVE);
    if caller==15&&live{
        static AUDITED:core::sync::atomic::AtomicBool=core::sync::atomic::AtomicBool::new(false);
        if !AUDITED.swap(true,core::sync::atomic::Ordering::Relaxed){
            sex_pdx::serial_record!("[files.browser.import.profile] target=15 task=15 slot=40 surface=300 create_only=1 held_storage_route=1 runtime_mint=0");
        }
    }
    live
}
