//! Recorder-only policy over held BOOT routes and existing Files mechanisms.
//! Single Files main thread owns this state; nested block waits only defer IPC.
use crate::{
    fs_service as fs, recording_image as image,
    recording_journal::{Fence, Journal},
    recording_role::{Check, Role, Scope, ScopedReply},
};
use sex_pdx::{app_lifecycle as life, crash_log, desktop_recording as wire, fs as f};
const SHELL_ROUTE: u64 = 6; // Added explicitly by recorder BOOT wiring, never raw PD lookup.
struct Runtime {
    ready: bool,
    role: Role,
    binding: Option<wire::BindRequest>,
    applied: Option<Scope>,
    data: ScopedReply,
    image: image::Service,
    image_refusal: Option<(wire::ImageRequest, wire::ImageReply)>,
}
impl Runtime {
    const fn new() -> Self {
        Self {
            ready: false,
            role: Role::new(),
            binding: None,
            applied: None,
            data: ScopedReply::new(),
            image: image::Service::new(),
            image_refusal: None,
        }
    }
}
static mut STATE: Runtime = Runtime::new();
static JOURNAL: Journal = Journal::new();
use core::sync::atomic::{AtomicBool, Ordering};
static ENTERED: AtomicBool = AtomicBool::new(false);
/// The sole Files main loop owns each turn. A future accidental nested dispatcher
/// fails before making a second mutable reference, including during disk waits.
struct Turn {
    _local: core::marker::PhantomData<*mut ()>,
}
impl Turn {
    fn enter() -> Self {
        assert!(
            ENTERED
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok(),
            "RECORD_RUNTIME_REENTRY"
        );
        Self {
            _local: core::marker::PhantomData,
        }
    }
    fn get(&mut self) -> &mut Runtime {
        unsafe { &mut *core::ptr::addr_of_mut!(STATE) }
    }
}
impl Drop for Turn {
    fn drop(&mut self) {
        ENTERED.store(false, Ordering::Release);
    }
}
/// Entry runs only after BOOT or kernel restart rearm (old owner Dead). Keep the
/// dedicated page before any lease admission; reset all nonpersistent authority.
pub fn initialize() {
    if !wire::ENABLED {
        return;
    }
    unsafe {
        core::ptr::write(core::ptr::addr_of_mut!(STATE), Runtime::new());
    }
    ENTERED.store(false, Ordering::Release);
    let mut turn = Turn::enter();
    let r = turn.get();
    fs::install_recording_binding(None);
    if !crash_log::keep(JOURNAL.address(), 1) {
        r.image.quarantine(0);
        sex_pdx::serial_record!("[record.image.unavailable] reason=keep");
        return;
    }
    let recovered = match JOURNAL.load() {
        Ok(fence) => Some(fence),
        Err(_) if files_epoch() == Some(1) && JOURNAL.all_zero() => {
            JOURNAL.initialize_cold().ok().map(|_| Fence::Free)
        }
        Err(_) => None,
    };
    if let Some(fence) = recovered {
        r.image = image::Service::restore(fence);
        r.ready = true;
    } else {
        let epoch = known_shell_epoch();
        r.image.quarantine(epoch);
        r.ready = JOURNAL.quarantine(epoch).is_ok();
        sex_pdx::serial_record!("[record.image.quarantined] reason=journal epoch={}", epoch);
    }
    sex_pdx::serial_record!(
        "[record.image.journal] files_epoch={} ready={} held={}",
        files_epoch().unwrap_or(0),
        r.ready as u8,
        r.image.busy() as u8
    );
}
fn known_shell_epoch() -> u64 {
    crash_log::peer_state(SHELL_ROUTE)
        .ok()
        .map_or(0, |peer| peer.epoch)
}
fn persist(r: &mut Runtime) -> bool {
    if JOURNAL.checkpoint(r.image.fence()).is_ok() {
        return true;
    }
    let epoch = known_shell_epoch();
    r.image.quarantine(epoch);
    r.ready = JOURNAL.quarantine(epoch).is_ok();
    sex_pdx::serial_record!(
        "[record.image.quarantined] reason=checkpoint epoch={}",
        epoch
    );
    false
}
fn poll_image(r: &mut Runtime) {
    if !r.ready {
        return;
    }
    if matches!(r.image.fence(), Fence::Quarantined(_)) {
        if let Ok(peer) = crash_log::peer_state(SHELL_ROUTE) {
            r.image
                .observe_quarantine(peer.epoch, peer.life == crash_log::PeerLife::Dead);
            let _ = persist(r);
        }
        return;
    }
    if let Some(owner) = r.image.owner() {
        r.image.poll(image_owner(owner), |reply| {
            let fence = if reply.kind == wire::ImageKind::Release {
                Fence::Releasing(owner)
            } else {
                Fence::Held(owner)
            };
            JOURNAL.checkpoint(fence).is_ok() && sex_pdx::pdx_reply(wire::SHELL, reply.word()) == 0
        });
        let _ = persist(r);
    }
}
fn files_epoch() -> Option<u32> {
    u32::try_from(crash_log::self_epoch().ok()?).ok().filter(|epoch| *epoch != 0)
}
fn target(task: u32) -> Check {
    if task == 0 {
        return Check::Invalid;
    }
    let Some(row) = sex_pdx::app_catalog::by_target(wire::TARGET) else {
        return Check::Invalid;
    };
    let catalog = match life::catalog(row.lifecycle_slot) {
        Ok(c) => c,
        Err(life::RETRY) => return Check::Wait,
        Err(_) => return Check::Invalid,
    };
    if !catalog.is_subject(wire::TARGET, row.surface, wire::SHELL)
        || !catalog.is_sender(wire::TARGET, task)
    {
        return Check::Invalid;
    }
    match life::info(row.lifecycle_slot) {
        Err(life::LIVE) => Check::Live,
        Err(life::RETRY) => Check::Wait,
        _ => Check::Invalid,
    }
}
fn recipient(scope: Scope) -> Check {
    if files_epoch() != Some(scope.files_epoch) {
        return Check::Invalid;
    }
    target(scope.task)
}
fn scope(scope: Scope) -> Check {
    if scope.shell_epoch == 0 || files_epoch() != Some(scope.files_epoch) {
        return Check::Invalid;
    }
    match crash_log::peer_state(SHELL_ROUTE) {
        Ok(peer) if peer.epoch == scope.shell_epoch && peer.live() => recipient(scope),
        Err(crash_log::RETRY) => Check::Wait,
        _ => Check::Invalid,
    }
}
fn image_owner(request: wire::ImageRequest) -> image::OwnerState {
    use image::OwnerState as S;
    match crash_log::peer_state(SHELL_ROUTE) {
        Ok(peer) if peer.epoch != request.shell_epoch => S::Quiescent,
        Ok(peer) if peer.live() && files_epoch() == Some(request.files_epoch) => S::Live,
        // A Files restart ends the old writer, not Shell's kernel image copy.
        // Only cleanup release or Shell epoch/Dead can surrender that obligation.
        Ok(peer) if peer.life == crash_log::PeerLife::Dead => S::Quiescent,
        Ok(_) => S::Closing,
        Err(_) => S::Unknown,
    }
}
fn apply(runtime: &mut Runtime) {
    let next = runtime.role.bound();
    if next != runtime.applied {
        fs::install_recording_binding(next.map(|s| s.task));
        runtime.applied = next;
    }
}
pub fn pending() -> bool {
    if !wire::ENABLED {
        return false;
    }
    let mut turn = Turn::enter();
    let r = turn.get();
    r.binding.is_some()
        || r.role.pending()
        || r.data.pending()
        || r.image.busy()
        || r.image_refusal.is_some()
}
pub fn image_busy() -> bool {
    if !wire::ENABLED {
        return false;
    }
    let mut turn = Turn::enter();
    turn.get().image.busy()
}
pub fn poll() {
    if !wire::ENABLED {
        return;
    }
    let mut turn = Turn::enter();
    let r = turn.get();
    r.role.reconcile(scope);
    apply(r);
    if let Some(request) = r.binding {
        match scope(Scope::request(request)) {
            Check::Wait => {}
            Check::Invalid => r.binding = None,
            Check::Live => {
                let status = fs::prepare_recording_root();
                match scope(Scope::request(request)) {
                    Check::Wait => {}
                    Check::Invalid => r.binding = None,
                    Check::Live => {
                        assert!(r.role.retain(request, status));
                        r.binding = None;
                    }
                }
            }
        }
    }
    r.role.poll(scope, |reply| {
        sex_pdx::pdx_reply(wire::SHELL, reply.word()) == 0
    });
    apply(r);
    r.data.poll(
        scope,
        recipient,
        |word| {
            let (seq, _, _) = f::decode_reply(word).expect("RECORD_FILES_REPLY");
            f::reply(seq, f::DENIED, 0)
        },
        |word| sex_pdx::pdx_reply(wire::TARGET, word) == 0,
    );
    poll_image(r);
    if let Some((request, reply)) = r.image_refusal {
        match image_owner(request) {
            image::OwnerState::Unknown => {}
            image::OwnerState::Quiescent => r.image_refusal = None,
            _ => {
                if sex_pdx::pdx_reply(wire::SHELL, reply.word()) == 0 {
                    r.image_refusal = None;
                }
            }
        }
    }
}
/// Exact old-task cleanup before ServiceCustody sends Files' retirement ACK.
pub fn retire(pd: u32, task: u32) {
    if !wire::ENABLED || pd != wire::TARGET {
        return;
    }
    let mut turn = Turn::enter();
    let r = turn.get();
    r.role.retire(task);
    r.data.retire(task);
    if r.binding.is_some_and(|request| request.task == task) {
        r.binding = None;
    }
    apply(r);
    let _ = crate::pdx::defer_retire_task(pd, task);
}
/// Runs after lifecycle notices, before ordinary Files/loader dispatch. Metadata
/// comes from live or retained stamped IPC, never from the request's data words.
pub fn message(msg: &crate::pdx::ReplayMsg) -> bool {
    if !wire::ENABLED {
        return false;
    }
    if matches!(msg.type_id, wire::OP_FILES_BIND | wire::OP_FILES_IMAGE) {
        if msg.caller_pd != wire::SHELL || msg.caller_task != wire::SHELL {
            return true;
        }
        let args = [msg.arg0, msg.arg1, msg.arg2];
        let mut turn = Turn::enter();
        let r = turn.get();
        if msg.type_id == wire::OP_FILES_BIND {
            let Some(request) = wire::BindRequest::decode(msg.caller_pd, args) else {
                return true;
            };
            if r.binding.is_some() || r.role.pending() {
                return true;
            }
            r.binding = Some(request);
            return true;
        }
        let Some(request) = wire::ImageRequest::decode(msg.caller_pd, args) else {
            return true;
        };
        // Reconcile an old owner before admitting a new epoch's transaction.
        poll_image(r);
        if !r.ready {
            if let Some(reply) = wire::ImageReply::new(request, wire::BindStatus::Unavailable, 0) {
                r.image_refusal = Some((request, reply));
            }
            return true;
        }
        match r.image.request(request, image_owner(request)) {
            image::Action::Retained => {}
            image::Action::Refused(reply) => {
                if r.image_refusal.is_none() {
                    r.image_refusal = Some((request, reply));
                }
            }
            image::Action::Read => {
                if !persist(r) {
                    return true;
                }
                let packed =
                    sex_pdx::program_store::pack_name(b"record").expect("RECORD_IMAGE_NAME");
                let result = sex_pdx::program_store::decode_reply(
                    crate::program_store::load_unbound(packed),
                );
                let (status, length) = match result {
                    Some(Ok(n)) if n > 0 && n <= wire::MAX_IMAGE_BYTES => (wire::BindStatus::Ok, n),
                    Some(Err(sex_pdx::program_store::NOT_FOUND)) => {
                        (wire::BindStatus::Unavailable, 0)
                    }
                    _ => (wire::BindStatus::Io, 0),
                };
                assert!(r.image.read_finished(request, status, length));
            }
        }
        let _ = persist(r);
        return true;
    }
    if msg.type_id != f::OP || msg.caller_pd != wire::TARGET {
        return false;
    }
    let mut turn = Turn::enter();
    let r = turn.get();
    if r.data.pending() {
        return true;
    }
    let Some(epoch) = files_epoch() else {
        return true;
    };
    let admitted = r
        .role
        .bound()
        .filter(|s| s.task == msg.caller_task)
        .unwrap_or(Scope {
            task: msg.caller_task,
            shell_epoch: 0,
            files_epoch: epoch,
        });
    let sequence = (msg.arg0 & 0xff_ffff) as u32;
    match recipient(admitted) {
        Check::Invalid => return true,
        Check::Wait => {
            assert!(r.data.retain(admitted, f::reply(sequence, f::PENDING, 0)));
            return true;
        }
        Check::Live => {}
    }
    let word = match scope(admitted) {
        Check::Live => fs::handle(wire::TARGET, msg.arg0, msg.arg1, msg.arg2),
        Check::Wait => f::reply(sequence, f::PENDING, 0),
        Check::Invalid => f::reply(sequence, f::DENIED, 0),
    };
    assert!(r.data.retain(admitted, word));
    true
}
