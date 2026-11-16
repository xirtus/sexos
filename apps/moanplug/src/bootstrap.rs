//! Pinned trusted bootstrap. There is no ambient/legacy launch fallback: failure
//! ends this helper before any loaded plugin instruction is executed.
use sex_pdx::{
    memory_region::{MemoryRegionInfo, RegionView, QUERY_VIEW, READ, WRITE},
    restricted_helper::Subject,
    restricted_job::Info,
};
pub fn scope(info: Info, own: Subject, files: u64, shell: u64) -> bool {
    info.child == Some(own)
        && info.parent.pd != own.pd
        && info.generation != 0
        && info.files_epoch == files
        && info.coordinator_epoch == shell
        && files != 0
        && shell != 0
        && info.output_hash == [0; 32]
}
pub fn shape(info: MemoryRegionInfo, view: RegionView, id: u32, text: bool) -> bool {
    info.region_id == id
        && info.peer_pd == Some(1)
        && info.rights == QUERY_VIEW | READ | WRITE
        && (if text {
            info.length == 8192
        } else {
            info.length >= 4 * 1024 * 1024 && info.length % (1024 * 1024) == 0
        })
        && info.length % 4096 == 0
        && view.region_id == id
        && view.length == info.length
        && view.virtual_address != 0
        && view.virtual_address % 4096 == 0
        && view.dma_address.is_none()
        && view
            .virtual_address
            .checked_add(view.length)
            .is_some_and(|end| end <= 1 << 47)
}
/// Caller has validated the full safely owned grant, including page padding.
pub unsafe fn erase(view: RegionView) {
    let base = view.virtual_address as *mut u64;
    for i in 0..view.length as usize / 8 {
        unsafe { core::ptr::write_volatile(base.add(i), 0) }
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}
pub fn receipt(op: u64, caller: u32, task: u32, value: u64, info: Info) -> bool {
    op == 1
        && caller == 3
        && task == 3
        && value == (0xd4 << 56 | (info.ticket as u64) << 32 | (info.generation & 0xffff_ffff))
}
#[cfg(target_os = "none")]
fn peer(slot: u64) -> Option<u64> {
    let p = sex_pdx::crash_log::peer_state(slot).ok()?;
    p.live().then_some(p.epoch)
}
#[cfg(target_os = "none")]
pub fn admit() -> Result<Info, ()> {
    let own = sex_pdx::app_lifecycle::catalog(sex_pdx::app_lifecycle::SELF_CATALOG_SLOT)
        .map_err(|_| ())?;
    let row = sex_pdx::app_catalog::by_target(own.target_pd)
        .filter(|r| r.app == sex_pdx::app_catalog::App::Program)
        .ok_or(())?;
    if !own.is_subject(row.target, row.surface, 3) {
        return Err(());
    }
    let files = peer(sex_pdx::SLOT_STORAGE).ok_or(())?;
    let shell = peer(sex_pdx::SLOT_SHELL).ok_or(())?;
    let info = sex_app::files::direct::restricted_boot().map_err(|_| ())?;
    if !scope(
        info,
        Subject {
            pd: own.target_pd,
            task: own.task_id,
        },
        files,
        shell,
    ) || peer(sex_pdx::SLOT_STORAGE) != Some(files)
        || peer(sex_pdx::SLOT_SHELL) != Some(shell)
    {
        return Err(());
    }
    let text = sex_pdx::text_region::index_of(own.target_pd).ok_or(())?;
    let pixel = sex_pdx::pixel_region::index_of(own.target_pd).ok_or(())?;
    for (slot, id, text) in [
        (
            sex_pdx::text_region::CLIENT_SLOT,
            sex_pdx::text_region::region_id(text),
            true,
        ),
        (
            sex_pdx::pixel_region::CLIENT_SLOT,
            sex_pdx::pixel_region::region_id(pixel),
            false,
        ),
    ] {
        let meta = sex_pdx::memory_region_info(slot).map_err(|_| ())?;
        let view = sex_pdx::memory_region_view(slot, 0, meta.length, QUERY_VIEW | READ | WRITE)
            .map_err(|_| ())?;
        if !shape(meta, view, id, text) {
            return Err(());
        }
        unsafe { erase(view) };
    }
    Ok(info)
}
#[cfg(target_os = "none")]
pub fn ready(info: Info) -> Result<(), ()> {
    if peer(sex_pdx::SLOT_STORAGE) != Some(info.files_epoch)
        || peer(sex_pdx::SLOT_SHELL) != Some(info.coordinator_epoch)
    {
        return Err(());
    }
    let nonce = core::array::from_fn::<_, 2, _>(|i| {
        u64::from_le_bytes(info.nonce[i * 8..i * 8 + 8].try_into().unwrap())
    });
    let until = sex_pdx::get_ticks().checked_add(5000).ok_or(())?;
    let mut sent = false;
    while sex_pdx::get_ticks() < until {
        if peer(sex_pdx::SLOT_STORAGE) != Some(info.files_epoch)
            || peer(sex_pdx::SLOT_SHELL) != Some(info.coordinator_epoch)
        {
            return Err(());
        }
        if !sent {
            sent = sex_pdx::pdx_call(
                sex_pdx::SLOT_SHELL,
                sex_pdx::restricted_helper::OP_READY,
                info.generation,
                nonce[0],
                nonce[1],
            )
            .0 == 0;
        }
        if let Some(m) = sex_pdx::pdx_try_listen_raw(0) {
            if sent && receipt(m.type_id, m.caller_pd, m.caller_task, m.arg0, info) {
                return if peer(sex_pdx::SLOT_STORAGE) == Some(info.files_epoch)
                    && peer(sex_pdx::SLOT_SHELL) == Some(info.coordinator_epoch)
                {
                    Ok(())
                } else {
                    Err(())
                };
            }
        }
        let _ = sex_pdx::sys_wait_self_ticks(1);
    }
    Err(())
}
