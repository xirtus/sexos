//! Serialized request/reply transport for services with one retained reply slot.
//! The enqueue result is not the service response. Callers must finish this
//! exchange before issuing the next reply-producing request.
/// Retry only queue backpressure from permanent boot services. Startup callers
/// have no outstanding response while enqueueing; ordinary incoming messages
/// are not consumed as replies to this not-yet-accepted request.
pub fn enqueue(slot: u64, opcode: u64, args: [u64; 3]) -> Result<(), u64> {
    loop {
        let (status, _) = sex_pdx::pdx_call(slot, opcode, args[0], args[1], args[2]);
        match status {
            0 => return Ok(()),
            sex_pdx::ERR_SERVICE_NOT_READY => {
                if let Some(message) = sex_pdx::pdx_try_listen_raw(0) {
                    if sex_pdx::app_lifecycle::close_requested(message.type_id,
                        message.caller_pd, [message.arg0, message.arg1, message.arg2], 15, 15)
                    { sex_pdx::task_finish(0); }
                }
                sex_pdx::sys_yield();
            }
            error => return Err(error),
        }
    }
}

pub fn request(slot: u64, caller: u32, opcode: u64, args: [u64; 3]) -> Result<u64, u64> {
    enqueue(slot, opcode, args)?;
    loop {
        let message = sex_pdx::pdx_listen_raw(0);
        if message.type_id == 1 && message.caller_pd == caller {
            return Ok(message.arg0);
        }
        if sex_pdx::app_lifecycle::close_requested(message.type_id, message.caller_pd,
            [message.arg0, message.arg1, message.arg2], 15, 15)
        { sex_pdx::task_finish(0); }
    }
}
