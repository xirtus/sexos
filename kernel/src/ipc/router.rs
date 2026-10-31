use crate::ipc::messages::{MessageType, IpcReply, IpcError};
use crate::ipc::DOMAIN_REGISTRY;
use crate::capabilities::engine::CapEngine;

/// Route a typed notification through the caller's held signal capability.
/// IPCtax: Lock-free asynchronous delivery via message rings.
pub fn route_signal(caller_pd_id: u32, target_pd_id: u32, signum: u8, cap_id: u64) -> Result<(), &'static str> {
    if !crate::app_resources::accepts_ipc(target_pd_id) {return Err("router: retired target");}
    // 1. Identify caller and target
    let caller_pd = DOMAIN_REGISTRY.get(caller_pd_id).ok_or("router: caller lost")?;
    let target_pd = DOMAIN_REGISTRY.get(target_pd_id).ok_or("router: target lost")?;

    // 2. Verify signal rights (RCU lookup)
    if !CapEngine::verify_signal_rights(caller_pd, cap_id) {
        return Err("router: denied");
    }

    // 3. Construct signal message
    let msg = MessageType::Signal(signum);

    // 4. Push to target's message ring
    unsafe {
        (*target_pd.message_ring).enqueue(msg).map_err(|_| "router: ring full")?;
    }

    // 5. Unpark the dedicated trampoline thread
    let trampoline_task = target_pd.trampoline_task.load(core::sync::atomic::Ordering::Acquire);
    if !trampoline_task.is_null() {
        crate::scheduler::unpark_thread(trampoline_task);
    }

    Ok(())
}

/// Deliver a value with the actual sender identity. This legacy target-directed
/// path is not a return capability; receivers must authenticate sender/protocol.
static REPLY_DENIED_RECORDS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
fn reply_denied(sender: u32, target: u32) {
    if REPLY_DENIED_RECORDS.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 16 {
        crate::serial_println!("[pdx.reply.denied] sender={} target={} reason=no_obligation", sender, target);
    }
}

pub fn send_reply(target_pd_id: u32, val: u64) -> Result<(), IpcError> {
    if !crate::app_resources::accepts_ipc(target_pd_id) {return Err(IpcError::InvalidTarget);}
    if let Some(pd) = DOMAIN_REGISTRY.get(target_pd_id) {
        let caller_pd = crate::core_local::CoreLocal::get().current_pd_ref().id;
        let caller_task = crate::ipc::sender::current_task(caller_pd)
            .ok_or(IpcError::InvalidCapability)?;
        // A target ID is not a return capability: only an accepted invocation
        // from that target obliges this domain to reply.
        let sender = crate::core_local::CoreLocal::get().current_pd_ref();
        let mut credits = sender.reply_credits.lock();
        if !credits.holds(target_pd_id) {
            drop(credits);
            reply_denied(caller_pd, target_pd_id);
            return Err(IpcError::InvalidCapability);
        }
        // 1. Enqueue the value into the target's internal reply slot
        // Keep the earlier reply until consumed. Sext retries a full ACK slot.
        let mut replies = pd.incoming_replies.lock();
        if replies.len() >= 1 {
            return Err(IpcError::BufferFull);
        }
        replies.push_back(IpcReply { value: val, caller_pd, caller_task });
        drop(replies);
        credits.consume(target_pd_id);
        drop(credits);

        // 2. Unpark the main task to trigger immediate processing
        let task_handle = pd.main_task.load(core::sync::atomic::Ordering::Acquire);
        if !task_handle.is_null() {
            crate::scheduler::unpark_thread(task_handle);
        }
        Ok(())
    } else {
        Err(IpcError::InvalidTarget)
    }
}
