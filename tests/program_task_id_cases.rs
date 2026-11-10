// Appended inside the production app_resources module with the source-pin host
// fixture helpers. Registry, frames and scheduler are explicit finite doubles.
#[test]
fn actual_counter_exhaustion_is_permanent_and_concurrent_ids_are_unique() {
    use std::collections::BTreeSet;
    NEXT_RUN_TASK.store(RUN_TASK_END - 64, Ordering::Release);
    let threads: Vec<_> = (0..8).map(|_| std::thread::spawn(|| {
        (0..32).filter_map(|_| next_run_task().ok()).collect::<Vec<_>>()
    })).collect();
    let ids: Vec<_> = threads.into_iter().flat_map(|thread| thread.join().unwrap()).collect();
    assert_eq!(ids.len(), 64);
    assert_eq!(ids.iter().copied().collect::<BTreeSet<_>>().len(), 64);
    assert!(ids.iter().all(|id| (RUN_TASK_END - 64..RUN_TASK_END).contains(id)));
    for _ in 0..512 { assert_eq!(next_run_task(), Err(DENIED)); }
    assert_eq!(NEXT_RUN_TASK.load(Ordering::Acquire), RUN_TASK_END);
    NEXT_RUN_TASK.store(u32::MAX, Ordering::Release);
    for _ in 0..512 { assert_eq!(next_run_task(), Err(DENIED)); }
    assert_eq!(NEXT_RUN_TASK.load(Ordering::Acquire), u32::MAX);
    for invalid in [0, 42, 0x3fff_ffff, RUN_TASK_END + 1, u32::MAX] {
        NEXT_RUN_TASK.store(invalid, Ordering::Release);
        for _ in 0..8 { assert_eq!(next_run_task(), Err(DENIED)); }
        assert_eq!(NEXT_RUN_TASK.load(Ordering::Acquire), invalid);
    }
}

#[test]
fn actual_rebirth_exhaustion_preserves_original_task_and_resource_state() {
    let pd = ProtectionDomain::new_key(16, 5);
    unsafe { (*pd.main_task.load(Ordering::Acquire)).state.exit(); }
    {
        let mut apps = APPS.lock();
        let app = &mut apps[1]; *app = App::new();
        app.target = 16; app.task = 16; app.boot_task = 16;
        app.main = pd.main_task.load(Ordering::Acquire) as usize;
        app.trampoline = pd.trampoline_task.load(Ordering::Acquire) as usize;
        app.ready = true; app.reusable = true; app.activated = true;
        app.retired = true; app.acked = REQUIRED; app.generation = 7;
        app.arena = 0x8000_0000; app.arena_pages = 8;
    }
    NEXT_RUN_TASK.store(u32::MAX, Ordering::Release);
    let before = snapshot(pd);
    for _ in 0..8 {
        assert_eq!(rebirth(&mut APPS.lock(), 1, None, None), Err(DENIED));
        assert_eq!(snapshot(pd), before, "exhaustion changed task/generation/manifest/arena/leaves/receipts");
    }
    assert_eq!(unsafe { (*pd.main_task.load(Ordering::Acquire)).id }, 16);
}

#[test]
fn actual_worker_exhaustion_preserves_run_and_counter() {
    let pd = ProtectionDomain::new_key(16, 5);
    {
        let mut apps = APPS.lock(); let app = &mut apps[1]; *app = App::new();
        app.target = 16; app.task = 16; app.boot_task = 16;
        app.main = pd.main_task.load(Ordering::Acquire) as usize;
        app.trampoline = pd.trampoline_task.load(Ordering::Acquire) as usize;
        app.ready = true; app.reusable = true; app.activated = true;
        app.arena = 0x8000_0000; app.arena_pages = 8;
    }
    NEXT_RUN_TASK.store(u32::MAX, Ordering::Release);
    let before = snapshot(pd);
    for _ in 0..8 {
        assert_eq!(spawn_worker(pd, 0x1000, 0x2000, 0), Err(DENIED));
        assert_eq!(snapshot(pd), before);
    }
}
