use desktop_recording as w;
use recording_journal::Fence;
fn reset() {
    recording_runtime::cold_reset_for_test();
    while pdx::defer_pop().is_some() {}
    *WORLD.lock().unwrap() = Some(World {
        files_epoch: 1,
        shell_epoch: 1,
        shell_life: crash_wire::PeerLife::Live,
        peer_error: None,
        task: 107,
        target_live: true,
        target_wait: false,
        keep_allowed: true,
        keeps: 0,
        capacity: 100,
        sent: vec![],
        requests: VecDeque::new(),
        nested: vec![],
        io_count: 0,
        prepare_count: 0,
        ordinary_loads: 0,
        bindings: vec![],
        data_calls: 0,
        on_io: None,
        on_send: None,
        root_status: w::BindStatus::Ok,
    });
    recording_runtime::initialize();
}
fn image_request(files_epoch: u32) -> w::ImageRequest {
    w::ImageRequest::new(
        w::ImageKind::Read,
        w::RoleSequence::new(1, 1).unwrap(),
        1,
        files_epoch,
    )
    .unwrap()
}
fn msg(op: u64, caller: u32, task: u32, args: [u64; 3]) -> pdx::ReplayMsg {
    pdx::ReplayMsg {
        type_id: op,
        caller_pd: caller,
        caller_task: task,
        arg0: args[0],
        arg1: args[1],
        arg2: args[2],
    }
}
fn image(request: w::ImageRequest) -> pdx::ReplayMsg {
    msg(w::OP_FILES_IMAGE, 3, 3, request.args())
}
fn release() -> pdx::ReplayMsg {
    let mut request = image_request(1);
    request.kind = w::ImageKind::Release;
    image(request)
}
fn replay() -> pdx::ReplayMsg {
    let packet = pdx::defer_pop().unwrap();
    let (op, caller, a, b, c) = packet.message;
    msg(op, caller, packet.caller_task, [a, b, c])
}
fn dispatch(msg: pdx::ReplayMsg) {
    if recording_runtime::message(&msg) {
        return;
    }
    let caller = msg.caller_pd;
    include!(env!("RECORDING_ORDINARY_DISPATCH_SOURCE"));
}
fn bind() -> w::BindRequest {
    w::BindRequest::new(w::RoleSequence::new(1, 2).unwrap(), 107, 1, 1).unwrap()
}
fn data(task: u32) -> pdx::ReplayMsg {
    msg(
        fs::OP,
        42,
        task,
        [
            fs::Request {
                seq: 17,
                kind: fs::WRITE,
                path_len: 4,
                offset: 0,
                length: 32,
            }
            .args()[0],
            0,
            32,
        ],
    )
}
#[test]
fn runtime_checkpoints_before_io_and_never_reenters_through_real_nested_wait() {
    reset();
    WORLD.lock().unwrap().as_mut().unwrap().on_io = Some(|| {
        assert_eq!(
            recording_runtime::journal_for_test(),
            Ok(Fence::Held(image_request(1)))
        );
    });
    let ordinary = program_store::pack_name(b"ordinary").unwrap();
    WORLD
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .nested
        .push(Message {
            type_id: program_store::OP,
            caller_pd: 3,
            caller_task: 3,
            arg0: ordinary[0],
            arg1: ordinary[1],
            arg2: 0,
        });
    dispatch(image(image_request(1)));
    assert!(recording_runtime::image_busy());
    assert!(!pdx::defer_empty());
    dispatch(replay());
    assert_eq!(
        WORLD.lock().unwrap().as_ref().unwrap().ordinary_loads,
        0,
        "queued ordinary write must not overwrite the leased image"
    );
    recording_runtime::poll();
    dispatch(release());
    recording_runtime::poll();
    assert!(!recording_runtime::image_busy());
    dispatch(msg(program_store::OP, 3, 3, [ordinary[0], ordinary[1], 0]));
    assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().ordinary_loads, 1);
}
#[test]
fn ordinary_load_finishes_before_a_record_load_deferred_by_its_real_wait() {
    reset();
    let request = image_request(1);
    let args = request.args();
    WORLD
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .nested
        .push(Message {
            type_id: w::OP_FILES_IMAGE,
            caller_pd: 3,
            caller_task: 3,
            arg0: args[0],
            arg1: args[1],
            arg2: args[2],
        });
    let name = program_store::pack_name(b"ordinary").unwrap();
    dispatch(msg(program_store::OP, 3, 3, [name[0], name[1], 0]));
    assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().ordinary_loads, 1);
    assert!(!recording_runtime::image_busy());
    dispatch(replay());
    assert!(recording_runtime::image_busy());
    assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().io_count, 2);
}
#[test]
fn runtime_rejects_wrong_sender_task_mode_and_epoch_before_any_read() {
    reset();
    let request = image_request(1);
    let args = request.args();
    for bad in [
        msg(w::OP_FILES_IMAGE, 12, 12, args),
        msg(w::OP_FILES_IMAGE, 3, 0x80000003, args),
        msg(
            w::OP_FILES_IMAGE,
            3,
            3,
            [args[0] | (1 << 24), args[1], args[2]],
        ),
        image(w::ImageRequest {
            shell_epoch: 2,
            ..request
        }),
        image(image_request(2)),
    ] {
        dispatch(bad);
        recording_runtime::poll();
    }
    assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().io_count, 0);
    assert!(!recording_runtime::image_busy());
}
#[test]
fn release_ack_backpressure_keeps_ordinary_loader_blocked_until_delivery() {
    reset();
    dispatch(image(image_request(1)));
    recording_runtime::poll();
    dispatch(release());
    WORLD.lock().unwrap().as_mut().unwrap().capacity = 0;
    recording_runtime::poll();
    assert!(recording_runtime::image_busy());
    assert_eq!(
        recording_runtime::journal_for_test(),
        Ok(Fence::Releasing(image_request(1)))
    );
    WORLD.lock().unwrap().as_mut().unwrap().capacity = 100;
    recording_runtime::poll();
    assert!(!recording_runtime::image_busy());
    assert_eq!(recording_runtime::journal_for_test(), Ok(Fence::Free));
}
#[test]
fn files_restart_after_positive_delivery_retains_fence_without_read_permission() {
    reset();
    dispatch(image(image_request(1)));
    recording_runtime::poll();
    {
        let mut world = WORLD.lock().unwrap();
        let w = world.as_mut().unwrap();
        w.files_epoch = 2;
        w.keeps = 0;
        w.sent.clear();
    }
    recording_runtime::initialize();
    recording_runtime::poll();
    assert!(recording_runtime::image_busy());
    assert!(WORLD.lock().unwrap().as_ref().unwrap().sent.is_empty());
    dispatch(release());
    recording_runtime::poll();
    assert!(!recording_runtime::image_busy());
    assert!(WORLD.lock().unwrap().as_ref().unwrap().sent.iter().all(|(_,word)|matches!(w::ImageReply::decode(w::FILES,*word),Some(reply)if reply.kind==w::ImageKind::Release)));
}
#[test]
fn crash_after_cleanup_send_before_clear_restores_only_exact_cleanup_ack() {
    reset();
    dispatch(image(image_request(1)));
    recording_runtime::poll();
    dispatch(release());
    WORLD.lock().unwrap().as_mut().unwrap().on_send = Some(|| panic!("crash after reply enqueue"));
    assert!(std::panic::catch_unwind(recording_runtime::poll).is_err());
    assert_eq!(
        recording_runtime::journal_for_test(),
        Ok(Fence::Releasing(image_request(1)))
    );
    {
        let mut world = WORLD.lock().unwrap();
        let w = world.as_mut().unwrap();
        w.files_epoch = 2;
        w.keeps = 0;
        w.sent.clear();
    }
    recording_runtime::initialize();
    recording_runtime::poll();
    assert!(!recording_runtime::image_busy());
    let sent = WORLD.lock().unwrap().as_ref().unwrap().sent.clone();
    assert_eq!(sent.len(), 1);
    let reply = w::ImageReply::decode(w::FILES, sent[0].1).unwrap();
    assert_eq!(reply.kind, w::ImageKind::Release);
    assert_eq!(reply.sequence, image_request(1).sequence);
}
#[test]
fn closing_same_shell_epoch_is_not_copy_quiescence_but_real_dead_is() {
    reset();
    dispatch(image(image_request(1)));
    recording_runtime::poll();
    WORLD.lock().unwrap().as_mut().unwrap().shell_life = crash_wire::PeerLife::Ending;
    recording_runtime::poll();
    assert!(recording_runtime::image_busy());
    WORLD.lock().unwrap().as_mut().unwrap().shell_life = crash_wire::PeerLife::Exited;
    recording_runtime::poll();
    assert!(recording_runtime::image_busy());
    WORLD.lock().unwrap().as_mut().unwrap().shell_life = crash_wire::PeerLife::Dead;
    recording_runtime::poll();
    assert!(!recording_runtime::image_busy());
    assert_eq!(recording_runtime::journal_for_test(), Ok(Fence::Free));
}
#[test]
fn role_bind_and_data_reply_backpressure_revalidate_epochs_and_cleanup_exact_task() {
    reset();
    dispatch(msg(w::OP_FILES_BIND, 3, 3, bind().args()));
    WORLD.lock().unwrap().as_mut().unwrap().capacity = 0;
    recording_runtime::poll();
    assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().bindings, [None]);
    WORLD.lock().unwrap().as_mut().unwrap().capacity = 100;
    recording_runtime::poll();
    assert_eq!(
        WORLD.lock().unwrap().as_ref().unwrap().bindings.last(),
        Some(&Some(107))
    );
    dispatch(data(107));
    {
        let mut world = WORLD.lock().unwrap();
        let w = world.as_mut().unwrap();
        w.shell_epoch = 2;
        w.sent.clear();
    }
    recording_runtime::poll();
    let replies = WORLD.lock().unwrap().as_ref().unwrap().sent.clone();
    assert!(replies
        .iter()
        .any(|(pd, word)| *pd == 42 && fs::decode_reply(*word) == Some((17, fs::DENIED, 0))));
    assert_eq!(
        WORLD.lock().unwrap().as_ref().unwrap().bindings.last(),
        Some(&None)
    );
    recording_runtime::retire(42, 107);
}
#[test]
fn unknown_target_holds_custody_and_old_stamped_task_never_executes_after_rebind() {
    reset();
    dispatch(msg(w::OP_FILES_BIND, 3, 3, bind().args()));
    recording_runtime::poll();
    WORLD.lock().unwrap().as_mut().unwrap().target_wait = true;
    dispatch(data(107));
    recording_runtime::poll();
    assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().data_calls, 0);
    WORLD.lock().unwrap().as_mut().unwrap().target_wait = false;
    recording_runtime::poll();
    recording_runtime::retire(42, 107);
    WORLD.lock().unwrap().as_mut().unwrap().task = 108;
    dispatch(data(107));
    recording_runtime::poll();
    assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().data_calls, 0);
}
#[test]
fn guarded_turn_rejects_accidental_nested_dispatch_before_alias_creation() {
    reset();
    WORLD.lock().unwrap().as_mut().unwrap().on_io = Some(|| {
        let _ = recording_runtime::pending();
    });
    assert!(std::panic::catch_unwind(|| dispatch(image(image_request(1)))).is_err());
    assert_eq!(
        recording_runtime::journal_for_test(),
        Ok(Fence::Held(image_request(1)))
    );
}

#[test]
fn cleanup_deferred_by_unknown_peer_checkpoints_before_later_ack() {
    reset();
    dispatch(image(image_request(1)));
    recording_runtime::poll();
    WORLD.lock().unwrap().as_mut().unwrap().peer_error = Some(crash_wire::RETRY);
    dispatch(release());
    assert!(recording_runtime::image_busy());
    assert_eq!(
        recording_runtime::journal_for_test(),
        Ok(Fence::Held(image_request(1)))
    );
    WORLD.lock().unwrap().as_mut().unwrap().peer_error = None;
    recording_runtime::poll();
    assert_eq!(recording_runtime::journal_for_test(), Ok(Fence::Free));
}
#[test]
fn retirement_removes_both_retained_data_result_and_deferred_old_task_before_ack() {
    reset();
    dispatch(msg(w::OP_FILES_BIND, 3, 3, bind().args()));
    recording_runtime::poll();
    let old = data(107);
    WORLD
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .nested
        .push(Message {
            type_id: old.type_id,
            caller_pd: old.caller_pd,
            caller_task: old.caller_task,
            arg0: old.arg0,
            arg1: old.arg1,
            arg2: old.arg2,
        });
    dispatch(data(107));
    assert!(!pdx::defer_empty());
    assert!(recording_runtime::pending());
    recording_runtime::retire(42, 107);
    assert!(pdx::defer_empty());
    assert!(!recording_runtime::pending());
}
