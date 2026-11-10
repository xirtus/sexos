use crate::serial_println;
use x86_64::VirtAddr;

pub static mut SEXDISPLAY_PD_ID: u32 = 0;
const SLOT_USB_SEXINPUT: u64 = 9;

// Pure typed INFO over actual BOOT table readback, never a fabricated runtime context.
fn browser_source_task(pd:&crate::capability::ProtectionDomain)->u32 {
    let task=pd.main_task.load(core::sync::atomic::Ordering::Acquire);
    assert!(!task.is_null(),"BOOT_SOURCE_MAIN_TASK");
    assert_eq!(unsafe{(*task).context.pd_ptr},pd as *const _,"BOOT_SOURCE_TASK_CONTEXT");
    let id=unsafe{(*task).id};assert!(id!=0,"BOOT_SOURCE_TASK_ZERO");id
}
fn browser_source_grants(holder:&crate::capability::ProtectionDomain,plan:sex_pdx::browser_sources::Plan)
 ->(sex_pdx::entropy_source::Grant,sex_pdx::clock_source::Grant,sex_pdx::trust_source::Grant){
    let row=sex_pdx::app_catalog::by_surface(plan.target(),plan.surface()).expect("BOOT_SOURCE_CATALOG");
    assert_eq!(row.app,sex_pdx::app_catalog::App::Browser,"BOOT_SOURCE_FAMILY");
    assert_eq!(row.lifecycle_slot,plan.lifecycle_slot(),"BOOT_SOURCE_LIFECYCLE_SLOT");
    let consumer=crate::ipc::DOMAIN_REGISTRY.get(plan.target()).expect("BOOT_SOURCE_CONSUMER");
    let(ht,ct)=(browser_source_task(holder),browser_source_task(consumer));
    assert_eq!(crate::app_resources::status(holder,plan.lifecycle_slot() as u64,1),
        [0,plan.target() as u64,ct as u64,plan.surface(),3],"BOOT_SOURCE_HELD_CATALOG");
    (sex_pdx::entropy_source::Grant{holder:sex_pdx::entropy_source::Identity{pd:holder.id,task:ht},consumer:sex_pdx::entropy_source::Identity{pd:plan.target(),task:ct},source:sex_pdx::entropy_source::RDSEED},
     sex_pdx::clock_source::Grant{holder:sex_pdx::clock_source::Identity{pd:holder.id,task:ht},consumer:sex_pdx::clock_source::Identity{pd:plan.target(),task:ct},source:sex_pdx::clock_source::CMOS_UTC},
     sex_pdx::trust_source::Grant{holder:sex_pdx::trust_source::Identity{pd:holder.id,task:ht},consumer:sex_pdx::trust_source::Identity{pd:plan.target(),task:ct},profile:sex_pdx::trust_config::PROFILE})
}
fn verify_browser_source_grants(holder:&crate::capability::ProtectionDomain,plan:sex_pdx::browser_sources::Plan)->sex_pdx::trust_source::Info{
    use crate::capability::CapabilityData;
    let expected=browser_source_grants(holder,plan);
    let read=|raw|holder.find_capability(crate::capability_slot::decode_capability_slot(raw).expect("BOOT_SOURCE_SLOT_WIDTH")).expect("BOOT_SOURCE_READBACK").data;
    let CapabilityData::EntropySource(entropy)=read(plan.entropy_slot())else{panic!("BOOT_SOURCE_ENTROPY_KIND")};
    let CapabilityData::ClockSource(clock)=read(plan.clock_slot())else{panic!("BOOT_SOURCE_CLOCK_KIND")};
    let CapabilityData::TrustPolicy(trust)=read(plan.trust_slot())else{panic!("BOOT_SOURCE_TRUST_KIND")};
    assert_eq!(entropy,expected.0,"BOOT_SOURCE_ENTROPY_READBACK");
    assert_eq!(clock,expected.1,"BOOT_SOURCE_CLOCK_READBACK");
    assert_eq!(trust,expected.2,"BOOT_SOURCE_TRUST_READBACK");
    let available=unsafe{core::arch::x86_64::__cpuid(0).eax>=7&&core::arch::x86_64::__cpuid_count(7,0).ebx&(1<<18)!=0};
    let ei=sex_pdx::entropy_source::Info::decode(sex_pdx::entropy_source::invoke(Some(&entropy),expected.0.holder,sex_pdx::entropy_source::INFO,[0;3],||available,||panic!("BOOT_INFO_SAMPLED_ENTROPY"))).expect("BOOT_SOURCE_ENTROPY_INFO");
    assert_eq!(ei,sex_pdx::entropy_source::Info{consumer:expected.0.consumer,available},"BOOT_SOURCE_ENTROPY_METADATA");
    let ci=sex_pdx::clock_source::Info::decode(sex_pdx::clock_source::invoke(Some(&clock),expected.1.holder,sex_pdx::clock_source::INFO,[0;3],||panic!("BOOT_INFO_SAMPLED_CLOCK"))).expect("BOOT_SOURCE_CLOCK_INFO");
    assert_eq!(ci,sex_pdx::clock_source::Info{consumer:expected.1.consumer,source:sex_pdx::clock_source::CMOS_UTC},"BOOT_SOURCE_CLOCK_METADATA");
    let ti=sex_pdx::trust_source::Info::decode(sex_pdx::trust_source::invoke(Some(&trust),expected.2.holder,sex_pdx::trust_source::INFO,[0;3])).expect("BOOT_SOURCE_TRUST_INFO");
    assert_eq!(ti,sex_pdx::trust_source::Info{consumer:expected.2.consumer,profile:sex_pdx::trust_config::PROFILE,root_bytes:sex_pdx::trust_config::ROOT_DER.len(),version:sex_pdx::trust_config::VERSION},"BOOT_SOURCE_TRUST_METADATA");
    ti
}

/// BOOT-only service edge for a fresh reserved Browser, with exact registered
/// holder/service context checks and actual table readback before any admission.
fn grant_browser_successor_route(pd:&crate::capability::ProtectionDomain,slot:u64,service:u32){
    use crate::capability::CapabilityData;
    assert_eq!(crate::ipc::BOOT_PHASE.load(core::sync::atomic::Ordering::Acquire),crate::ipc::BootPhase::RegistryBuild as u8,"BOOT_BROWSER_ROUTE_PHASE");
    assert!(sex_pdx::app_catalog::by_target(pd.id).is_some_and(|row|row.app==sex_pdx::app_catalog::App::Browser&&row.ordinal!=0),"BOOT_BROWSER_ROUTE_SUBJECT");
    let task=browser_source_task(pd);
    let target=crate::ipc::DOMAIN_REGISTRY.get(service).expect("BOOT_BROWSER_ROUTE_SERVICE");
    let _=browser_source_task(target);
    let decoded=crate::capability_slot::decode_capability_slot(slot).expect("BOOT_BROWSER_ROUTE_SLOT");
    assert!(pd.find_capability(decoded).is_none(),"BOOT_BROWSER_ROUTE_COLLISION");
    pd.grant_capability(slot,CapabilityData::Domain(service));
    assert!(matches!(pd.find_capability(decoded).map(|cap|cap.data),Some(CapabilityData::Domain(actual))if actual==service),"BOOT_BROWSER_ROUTE_READBACK");
    serial_println!("[boot.browser.service.route] target={} task={} slot={} service={} checked=1 readback=1 runtime_mint=0",pd.id,task,slot,service);
}

pub fn init() {
    // Advance boot phase to allow registry insertion
    unsafe { crate::ipc::BOOT_CONTROLLER.advance(crate::ipc::BootPhase::RegistryBuild); }

    // Phase 1.5: Populate PRIMARY_GPU_LEASE exactly once
    {
        use crate::graphics::gpu_lease::{select_primary_gpu, PRIMARY_GPU_LEASE};
        if let Some(lease) = select_primary_gpu() {
            *PRIMARY_GPU_LEASE.lock() = Some(lease);
            serial_println!("init: PRIMARY_GPU_LEASE populated");
        }
    }

    let modules_res = crate::MODULE_REQUEST.response();
    if modules_res.is_none() {
        panic!("FATAL: Limine returned no modules. Check limine.cfg and ISO layout.");
    }
    let modules = modules_res.unwrap();
    serial_println!("init: Found {} Limine modules", modules.modules().len());

    let mut sexdisp_id = 0;
    let mut sexdrive_id = 0;
    let mut silkshell_id = 0;
    let mut sexinput_id = 0;
    let mut sexusb_id = 0;
    let mut silkbar_id = 0;
    let mut linen_id = 0;
    let mut sexstore_id = 0;
    let mut quil_id = 0;
    let mut sexbell_id = 0;
    let mut sexfiles_id = 0;
    let mut spindle_id = 0;
    let mut sexnet_id = 0;
    let mut kaleido_id = 0;
    let mut kaleido_domain_id = 0;
    let mut sext_id = 0;

    // Fixed Spawn Order (Deterministic IDs)
    let module_paths = ["sexdisplay", "sexdrive", "silk-shell", "sexinput", "sexusb", "silkbar", "linen", "sexstore", "quil", "sexbell", "sexfiles", "spindle", "sexnet", "sext", "kaleidoscope"];
    crate::app_resources::plan_boot_apps(&sex_pdx::app_catalog::TARGETS);
    for (i, target) in module_paths.iter().enumerate() {
        let domain_id = (i + 1) as u8;
        for module in modules.modules() {
            let path = module.path();
            if path.contains(target) {
                serial_println!("[bootgraph.pd.spawn.begin] pd={}", target);
                let key=sex_pdx::organs::service_key(domain_id);
                let spawned=if key!=domain_id {
                    crate::pd::create::create_boot_domain(path,None,domain_id,key)
                }else{pdx_spawn(path, domain_id)};
                match spawned {
                    Ok(id) => {
                        serial_println!("✓ Spawned PD {}: {} (Domain {})", id, path, domain_id);
                        serial_println!(
                            "[bootgraph.pd.spawn.ok] pd={} id={} pkey={}",
                            target, id, domain_id
                        );
                        if domain_id == 1 { 
                            sexdisp_id = id; 
                            unsafe { SEXDISPLAY_PD_ID = id; }
                            
                            use crate::graphics::gpu_lease::claim_primary_for_pd1;
                            let lease = claim_primary_for_pd1();
                            
                            use crate::ipc::DOMAIN_REGISTRY;
                            let pd_ptr = DOMAIN_REGISTRY.get(id).expect("PD1 not in registry");
                            let main_task_ptr = (*pd_ptr).main_task.load(core::sync::atomic::Ordering::Acquire);
                            if !main_task_ptr.is_null() {
                                let main_task = unsafe { &mut *main_task_ptr };
                                main_task.ext_init = Some(crate::scheduler::InitArg { display_lease: lease });
                            }
                        } else if domain_id == 2 {
                            sexdrive_id = id;
                        } else if domain_id == 3 {
                            silkshell_id = id;
                        } else if domain_id == 4 {
                            sexinput_id = id;
                        } else if domain_id == 5 {
                            sexusb_id = id;
                        } else if domain_id == 6 {
                            silkbar_id = id;
                        } else if domain_id == 7 {
                            linen_id = id;
                        } else if domain_id == 8 {
                            sexstore_id = id;
                            serial_println!("[kernel.sexstore.spawn] id={}", id);
                        } else if domain_id == 9 {
                            quil_id = id;
                            serial_println!("[kernel.spawn.quil] id={} path={}", id, path);
                        } else if domain_id == 10 {
                            sexbell_id = id;
                            serial_println!("[kernel.spawn.sexbell] id={} path={}", id, path);
                        } else if domain_id == 11 {
                            sexfiles_id = id;
                            serial_println!("[kernel.spawn.sexfiles] id={} path={}", id, path);
                        } else if domain_id == 12 {
                            spindle_id = id;
                            serial_println!("[kernel.spawn.spindle] id={} path={}", id, path);
                        } else if domain_id == 13 {
                            sexnet_id = id;
                            serial_println!("[kernel.spawn.sexnet] id={} path={}", id, path);
                        } else if domain_id == 14 {
                            sext_id = id;
                            serial_println!("[kernel.spawn.sext] id={} domain_id={}", id, domain_id);
                        } else if domain_id == 15 {
                            kaleido_id = id;
                            kaleido_domain_id = domain_id;
                            serial_println!("[kernel.spawn.kaleidoscope] id={} domain_id={}", id, domain_id);
                        }
                    }
                    Err(e) => {
                        serial_println!("!! Spawn Error {}: {}", path, e);
                        serial_println!("[bootgraph.pd.spawn.err] pd={} reason=spawn_failed", target);
                    }
                }
                break;
            }
        }
    }

    // Every BOOT-reserved successor incarnation in the app catalog, each with
    // its own layout; the boot incarnation's hardware key is reused only after
    // that predecessor has no private translations/custody.
    use sex_pdx::app_catalog::{self as apps, App};
    assert_eq!((quil_id,spindle_id,kaleido_id),(apps::boot_incarnation(App::Quil).target,apps::boot_incarnation(App::Spindle).target,apps::boot_incarnation(App::Browser).target),"BOOT_CATALOG_SPAWN_ORDER");
    for incarnation in apps::INCARNATIONS.iter().filter(|i|i.ordinal!=0||i.app==App::Program) {
        let name=match incarnation.app{App::Quil=>"quil",App::Spindle=>"spindle",App::Browser=>"kaleidoscope",App::Program=>"hello"};
        let module=modules.modules().iter().find(|m|m.path().contains(name)).expect("BOOT_SUCCESSOR_MODULE");
        let key=if incarnation.app==App::Program{sex_pdx::organs::program_key(incarnation.ordinal) as u32}else{apps::boot_incarnation(incarnation.app).target};
        let (layout,pkey)=(u8::try_from(incarnation.target).expect("BOOT_SUCCESSOR_LAYOUT"),u8::try_from(key).expect("BOOT_SUCCESSOR_KEY"));
        let created=crate::pd::create::create_boot_domain(module.path(),Some(incarnation.target),layout,pkey).expect("BOOT_SUCCESSOR");
        assert_eq!(created,incarnation.target,"BOOT_SUCCESSOR_IDENTITY");
    }

    // Grant Phase 25 well-known capabilities
    serial_println!("[bootgraph.phase25.begin]");
    // Static BOOT_DAG return edges for authenticated display-ready messages.
    // Runtime code only invokes these capabilities; it cannot mint routes.
    if sexdisp_id != 0 {
        assert_eq!(sexdisp_id, sex_pdx::display_ready::DISPLAY_PD_ID,
            "DISPLAY_READY_GENESIS_ID_MISMATCH");
        if let Some(pd) = crate::ipc::DOMAIN_REGISTRY.get(sexdisp_id) {
            for (slot, target, slot_name) in [
                (sex_pdx::SLOT_SHELL, silkshell_id, "SLOT_SHELL"),
                (sex_pdx::SLOT_SILKBAR, silkbar_id, "SLOT_SILKBAR"),
            ] {
                if target != 0 {
                    pd.grant_capability(slot, crate::capability::CapabilityData::Domain(target));
                    serial_println!(
                        "[bootgraph.cap.grant from=kernel to={} slot={} target={} ok=1]",
                        sexdisp_id, slot_name, target
                    );
                }
            }
        }
    }
    // Static BOOT_DAG shell-ready return edge. Private shell slot 9 is free;
    // slot 3 must retain the initial self-signal capability in this domain.
    if silkshell_id != 0 && sexinput_id != 0 {
        assert_eq!(silkshell_id, sex_pdx::shell_ready::SHELL_PD_ID,
            "SHELL_READY_GENESIS_ID_MISMATCH");
        assert_eq!(sexinput_id, sex_pdx::shell_ready::INPUT_PD_ID,
            "SHELL_READY_INPUT_ID_MISMATCH");
        let pd = crate::ipc::DOMAIN_REGISTRY.get(silkshell_id)
            .expect("SHELL_READY_GENESIS_SHELL_MISSING");
        assert!(pd.find_capability(sex_pdx::shell_ready::SLOT_SHELL_INPUT_READY as u32).is_none(),
            "SHELL_READY_INPUT_SLOT_COLLISION");
        pd.grant_capability(sex_pdx::shell_ready::SLOT_SHELL_INPUT_READY, crate::capability::CapabilityData::Domain(sexinput_id));
        serial_println!(
            "[bootgraph.cap.grant from=kernel to={} slot=SLOT_SHELL_INPUT_READY target={} ok=1]",
            silkshell_id, sexinput_id
        );
    }
    if sexdisp_id != 0 && silkshell_id != 0 {
        use crate::ipc::DOMAIN_REGISTRY;
        use crate::capability::CapabilityData;

        if let Some(pd) = DOMAIN_REGISTRY.get(silkshell_id) {
            pd.grant_capability(sex_pdx::SLOT_DISPLAY, CapabilityData::Domain(sexdisp_id));
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_DISPLAY target={} ok=1]",
                silkshell_id, sexdisp_id
            );
            serial_println!(
                "[bootgraph.cap.grant] from=kernel to={} slot=SLOT_DISPLAY target={} ok=1",
                silkshell_id, sexdisp_id
            );
            pd.grant_capability(sex_pdx::SLOT_SHELL,   CapabilityData::Domain(silkshell_id));
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_SHELL target={} ok=1]",
                silkshell_id, silkshell_id
            );
            serial_println!(
                "[bootgraph.cap.grant] from=kernel to={} slot=SLOT_SHELL target={} ok=1",
                silkshell_id, silkshell_id
            );
            // Stage 2B: silk-shell can send workspace IPC to SilkBar
            pd.grant_capability(sex_pdx::SLOT_SILKBAR, CapabilityData::Domain(silkbar_id));
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_SILKBAR target={} ok=1 optional=1]",
                silkshell_id, silkbar_id
            );
            serial_println!(
                "[bootgraph.cap.grant] from=kernel to={} slot=SLOT_SILKBAR target={} ok=1 optional=1",
                silkshell_id, silkbar_id
            );
            if sexstore_id != 0 {
                pd.grant_capability(sex_pdx::SLOT_SEXSTORE, CapabilityData::Domain(sexstore_id));
                serial_println!("[kernel.sexstore.cap] shell={} store={}", silkshell_id, sexstore_id);
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_SEXSTORE target={} ok=1 optional=1]",
                    silkshell_id, sexstore_id
                );
                serial_println!(
                    "[bootgraph.cap.grant] from=kernel to={} slot=SLOT_SEXSTORE target={} ok=1 optional=1",
                    silkshell_id, sexstore_id
                );
            } else {
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_SEXSTORE target=missing ok=0 optional=1 reason=sexstore_absent]",
                    silkshell_id
                );
                serial_println!(
                    "[bootgraph.cap.grant] from=kernel to={} slot=SLOT_SEXSTORE target=missing ok=0 optional=1 reason=missing_target",
                    silkshell_id
                );
            }
            // Bell read-cap: silk-shell can call OP_BELL_LIST
            if sexbell_id != 0 {
                pd.grant_capability(sex_pdx::SLOT_BELL, CapabilityData::Domain(sexbell_id));
                serial_println!("[kernel.sexbell.cap.shell] shell→bell slot=12");
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_BELL target={} ok=1 optional=1]",
                    silkshell_id, sexbell_id
                );
                serial_println!(
                    "[bootgraph.cap.grant] from=kernel to={} slot=SLOT_BELL target={} ok=1 optional=1",
                    silkshell_id, sexbell_id
                );
            } else {
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_BELL target=missing ok=0 optional=1 reason=sexbell_absent]",
                    silkshell_id
                );
                serial_println!(
                    "[bootgraph.cap.grant] from=kernel to={} slot=SLOT_BELL target=missing ok=0 optional=1 reason=missing_target",
                    silkshell_id
                );
            }
            serial_println!("✓ Phase 25: Capabilities granted to silk-shell");
        } else {
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_DISPLAY target={} ok=0 optional=1 reason=pd_missing]",
                silkshell_id, sexdisp_id
            );
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_SHELL target={} ok=0 optional=1 reason=pd_missing]",
                silkshell_id, silkshell_id
            );
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_SILKBAR target={} ok=0 optional=1 reason=pd_missing]",
                silkshell_id, silkbar_id
            );
            if sexstore_id != 0 {
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_SEXSTORE target={} ok=0 optional=1 reason=pd_missing]",
                    silkshell_id, sexstore_id
                );
            }
            if sexbell_id != 0 {
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_BELL target={} ok=0 optional=1 reason=pd_missing]",
                    silkshell_id, sexbell_id
                );
            }
        }

        if sexinput_id != 0 {
            if let Some(pd) = DOMAIN_REGISTRY.get(sexinput_id) {
                // Static Binding: SLOT_INPUT -> Kernel INPUT_RING
                pd.grant_capability(sex_pdx::SLOT_INPUT, CapabilityData::InputRing);
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_INPUT target=input_ring ok=1]",
                    sexinput_id
                );
                serial_println!(
                    "[bootgraph.cap.grant] from=kernel to={} slot=SLOT_INPUT target=input_ring ok=1",
                    sexinput_id
                );
                // Grant access to silk-shell for event forwarding
                pd.grant_capability(sex_pdx::SLOT_SHELL, CapabilityData::Domain(silkshell_id));
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_SHELL target={} ok=1]",
                    sexinput_id, silkshell_id
                );
                serial_println!(
                    "[bootgraph.cap.grant] from=kernel to={} slot=SLOT_SHELL target={} ok=1",
                    sexinput_id, silkshell_id
                );
                serial_println!("✓ Phase 25: Capabilities granted to sexinput");
            } else {
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_INPUT target=input_ring ok=0 optional=1 reason=pd_missing]",
                    sexinput_id
                );
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_SHELL target={} ok=0 optional=1 reason=pd_missing]",
                    sexinput_id, silkshell_id
                );
            }
        } else {
            serial_println!(
                "[bootgraph.cap.grant from=kernel to=missing slot=SLOT_INPUT target=input_ring ok=0 optional=1 reason=sexinput_absent]"
            );
            serial_println!(
                "[bootgraph.cap.grant from=kernel to=missing slot=SLOT_SHELL target={} ok=0 optional=1 reason=sexinput_absent]",
                silkshell_id
            );
        }

        // Minimal USB input route: allow sexusb to send decoded mouse reports
        // directly to sexinput over one dedicated domain slot.
        if sexusb_id != 0 && sexinput_id != 0 {
            if let Some(pd) = DOMAIN_REGISTRY.get(sexusb_id) {
                pd.grant_capability(SLOT_USB_SEXINPUT, CapabilityData::Domain(sexinput_id));
                serial_println!("✓ cap.route: sexusb->sexinput slot {}", SLOT_USB_SEXINPUT);
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot={} target={} ok=1 optional=1]",
                    sexusb_id, SLOT_USB_SEXINPUT, sexinput_id
                );
                serial_println!(
                    "[bootgraph.cap.grant] from=kernel to={} slot={} target={} ok=1 optional=1",
                    sexusb_id, SLOT_USB_SEXINPUT, sexinput_id
                );
            } else {
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot={} target={} ok=0 optional=1 reason=pd_missing]",
                    sexusb_id, SLOT_USB_SEXINPUT, sexinput_id
                );
            }
        } else {
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot={} target={} ok=0 optional=1 reason=sexusb_or_sexinput_absent]",
                sexusb_id, SLOT_USB_SEXINPUT, sexinput_id
            );
        }

    } else {
        serial_println!(
            "[bootgraph.cap.grant from=kernel to={} slot=SLOT_DISPLAY target={} ok=0 optional=1 reason=sexdisplay_or_silkshell_absent]",
            silkshell_id, sexdisp_id
        );
    }

    // Discrete GPUs no driver owns yet are powered off before leases are made.
    crate::hal::dgpu::init();

    // Hardware discovery and driver lease routing.
    // Includes XHCI discovery + lease to sexusb (slot SLOT_USB_HOST) when present.
    if sexdrive_id != 0 && sexdisp_id != 0 {
        crate::devmgr::init(sexdrive_id, sexdisp_id, sexusb_id, sexnet_id, sexbell_id);
    }

    // SilkBar delivery path: grant display capability independently of silk-shell.
    // Otherwise SilkBar updates are silently blocked whenever silk-shell is absent.
    if sexdisp_id != 0 && silkbar_id != 0 {
        use crate::ipc::DOMAIN_REGISTRY;
        use crate::capability::CapabilityData;
        if let Some(pd) = DOMAIN_REGISTRY.get(silkbar_id) {
            pd.grant_capability(sex_pdx::SLOT_DISPLAY, CapabilityData::Domain(sexdisp_id));
            serial_println!("✓ SilkBar v8: Capability SLOT_DISPLAY granted");
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_DISPLAY target={} ok=1]",
                silkbar_id, sexdisp_id
            );
            // Bell polling cap: SilkBar needs SLOT_BELL for OP_BELL_LIST.
            // This is a read-only LIST capability — SilkBar has no NOTIFY/CLOSE/ACTION.
            // Bell server-side allowlist (BELL_LIST_ALLOWLIST) provides second gate.
            if sexbell_id != 0 {
                pd.grant_capability(sex_pdx::SLOT_BELL, CapabilityData::Domain(sexbell_id));
                serial_println!("[kernel.sexbell.cap.silkbar] silkbar→bell slot=12");
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_BELL target={} ok=1 optional=1]",
                    silkbar_id, sexbell_id
                );
            } else {
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_BELL target=missing ok=0 optional=1 reason=sexbell_absent]",
                    silkbar_id
                );
            }
        } else {
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_DISPLAY target={} ok=0 optional=1 reason=pd_missing]",
                silkbar_id, sexdisp_id
            );
        }
    } else {
        serial_println!(
            "[bootgraph.cap.grant from=kernel to={} slot=SLOT_DISPLAY target={} ok=0 optional=1 reason=sexdisplay_or_silkbar_absent]",
            silkbar_id, sexdisp_id
        );
    }

    // Linen delivery path: grant display capability for placeholder surface.
    if linen_id != 0 && sexdisp_id != 0 {
        use crate::ipc::DOMAIN_REGISTRY;
        use crate::capability::CapabilityData;
        if let Some(pd) = DOMAIN_REGISTRY.get(linen_id) {
            pd.grant_capability(sex_pdx::SLOT_DISPLAY, CapabilityData::Domain(sexdisp_id));
            serial_println!("✓ Phase 25: Capability SLOT_DISPLAY granted to linen");
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_DISPLAY target={} ok=1]",
                linen_id, sexdisp_id
            );
            if sexfiles_id != 0 {
                pd.grant_capability(sex_pdx::SLOT_STORAGE, CapabilityData::Domain(sexfiles_id));
                serial_println!(
                    "[kernel.cap.storage.linen] slot={} target_pd={}",
                    sex_pdx::SLOT_STORAGE,
                    sexfiles_id
                );
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_STORAGE target={} ok=1 optional=1]",
                    linen_id, sexfiles_id
                );
            } else {
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_STORAGE target=missing ok=0 optional=1 reason=sexfiles_absent]",
                    linen_id
                );
            }
        } else {
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_DISPLAY target={} ok=0 optional=1 reason=pd_missing]",
                linen_id, sexdisp_id
            );
        }
        
        if silkshell_id != 0 {
            if let Some(pd) = DOMAIN_REGISTRY.get(silkshell_id) {
                pd.grant_capability(sex_pdx::SLOT_LINEN, CapabilityData::Domain(linen_id));
                serial_println!("[kernel.cap.linen.route] shell->linen slot={}", sex_pdx::SLOT_LINEN);
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_LINEN target={} ok=1 optional=1]",
                    silkshell_id, linen_id
                );
            } else {
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_LINEN target={} ok=0 optional=1 reason=pd_missing]",
                    silkshell_id, linen_id
                );
            }
        } else {
            serial_println!(
                "[bootgraph.cap.grant from=kernel to=missing slot=SLOT_LINEN target={} ok=0 optional=1 reason=silkshell_absent]",
                linen_id
            );
        }
    } else {
        serial_println!(
            "[bootgraph.cap.grant from=kernel to={} slot=SLOT_DISPLAY target={} ok=0 optional=1 reason=linen_or_sexdisplay_absent]",
            linen_id, sexdisp_id
        );
    }

    // Quil route: grant silk-shell capability to ping Quil (no display caps).
    if quil_id != 0 && silkshell_id != 0 {
        use crate::ipc::DOMAIN_REGISTRY;
        use crate::capability::CapabilityData;
        if let Some(pd) = DOMAIN_REGISTRY.get(silkshell_id) {
            pd.grant_capability(sex_pdx::SLOT_QUIL, CapabilityData::Domain(quil_id));
            serial_println!("[kernel.cap.quil.route] shell->quil slot={}", sex_pdx::SLOT_QUIL);
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_QUIL target={} ok=1 optional=1]",
                silkshell_id, quil_id
            );
        } else {
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_QUIL target={} ok=0 optional=1 reason=pd_missing]",
                silkshell_id, quil_id
            );
        }
        
        if sexdisp_id != 0 {
            if let Some(pd) = DOMAIN_REGISTRY.get(quil_id) {
                pd.grant_capability(sex_pdx::SLOT_DISPLAY, CapabilityData::Domain(sexdisp_id));
                serial_println!("✓ Capability SLOT_DISPLAY granted to quil");
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_DISPLAY target={} ok=1]",
                    quil_id, sexdisp_id
                );
                if sexfiles_id != 0 {
                    pd.grant_capability(sex_pdx::SLOT_STORAGE, CapabilityData::Domain(sexfiles_id));
                    serial_println!("[kernel.cap.storage.quil] quil->sexfiles slot={}", sex_pdx::SLOT_STORAGE);
                    serial_println!(
                        "[bootgraph.cap.grant from=kernel to={} slot=SLOT_STORAGE target={} ok=1 optional=1]",
                        quil_id, sexfiles_id
                    );
                } else {
                    serial_println!(
                        "[bootgraph.cap.grant from=kernel to={} slot=SLOT_STORAGE target=missing ok=0 optional=1 reason=sexfiles_absent]",
                        quil_id
                    );
                }
            } else {
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_DISPLAY target={} ok=0 optional=1 reason=pd_missing]",
                    quil_id, sexdisp_id
                );
            }
        } else {
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_DISPLAY target={} ok=0 optional=1 reason=sexdisplay_absent]",
                quil_id, sexdisp_id
            );
        }
    } else {
        serial_println!(
            "[bootgraph.cap.grant from=kernel to={} slot=SLOT_QUIL target={} ok=0 optional=1 reason=quil_or_silkshell_absent]",
            silkshell_id, quil_id
        );
    }

    // Quil document controls need the same immutable orchestration edge
    // already provisioned for its BOOT-reserved successor. Never mint at run time.
    if quil_id!=0 && silkshell_id!=0 {
        let pd=crate::ipc::DOMAIN_REGISTRY.get(quil_id).expect("BOOT_QUIL_MISSING");
        assert!(pd.find_capability(sex_pdx::SLOT_SHELL as u32).is_none(),"BOOT_QUIL_SHELL_SLOT_OCCUPIED");
        pd.grant_capability(sex_pdx::SLOT_SHELL,crate::capability::CapabilityData::Domain(silkshell_id));
        serial_println!("[boot.app.route] target={} slot={} service={} runtime_mint=0",quil_id,sex_pdx::SLOT_SHELL,silkshell_id);
    }

    // Spindle route: grant silk-shell capability to send key events
    // to Spindle terminal (domain 12) via SLOT_SPINDLE.
    if spindle_id != 0 && silkshell_id != 0 {
        use crate::ipc::DOMAIN_REGISTRY;
        use crate::capability::CapabilityData;
        if let Some(pd) = DOMAIN_REGISTRY.get(silkshell_id) {
            pd.grant_capability(sex_pdx::SLOT_SPINDLE, CapabilityData::Domain(spindle_id));
            serial_println!("[kernel.cap.spindle.route] shell->spindle slot={}", sex_pdx::SLOT_SPINDLE);
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_SPINDLE target={} ok=1 optional=1]",
                silkshell_id, spindle_id
            );
        } else {
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_SPINDLE target={} ok=0 optional=1 reason=pd_missing]",
                silkshell_id, spindle_id
            );
        }
    } else {
        serial_println!(
            "[bootgraph.cap.grant from=kernel to={} slot=SLOT_SPINDLE target={} ok=0 optional=1 reason=spindle_or_silkshell_absent]",
            silkshell_id, spindle_id
        );
    }

    // Bell self-cap: grant SLOT_BELL to sexbell for listen (no external caps).
    if sexbell_id != 0 {
        use crate::ipc::DOMAIN_REGISTRY;
        use crate::capability::CapabilityData;
        if let Some(pd) = DOMAIN_REGISTRY.get(sexbell_id) {
            pd.grant_capability(sex_pdx::SLOT_BELL, CapabilityData::Domain(sexbell_id));
            serial_println!("[kernel.sexbell.cap] self slot={}", sex_pdx::SLOT_BELL);
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_BELL target={} ok=1 optional=1]",
                sexbell_id, sexbell_id
            );
        } else {
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_BELL target={} ok=0 optional=1 reason=pd_missing]",
                sexbell_id, sexbell_id
            );
        }
    } else {
        serial_println!(
            "[bootgraph.cap.grant from=kernel to=missing slot=SLOT_BELL target=missing ok=0 optional=1 reason=sexbell_absent]"
        );
    }

    // Spindle capability grants: terminal console bridges (PD 12).
    if spindle_id != 0 {
        use crate::ipc::DOMAIN_REGISTRY;
        use crate::capability::CapabilityData;
        if let Some(pd) = DOMAIN_REGISTRY.get(spindle_id) {
            if sexdisp_id != 0 {
                pd.grant_capability(sex_pdx::SLOT_DISPLAY, CapabilityData::Domain(sexdisp_id));
                serial_println!("[kernel.cap.display.spindle] spindle->sexdisplay slot={}", sex_pdx::SLOT_DISPLAY);
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_DISPLAY target={} ok=1]",
                    spindle_id, sexdisp_id
                );
            } else {
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_DISPLAY target=missing ok=0 optional=1 reason=sexdisplay_absent]",
                    spindle_id
                );
            }
            if sexfiles_id != 0 {
                pd.grant_capability(sex_pdx::SLOT_STORAGE, CapabilityData::Domain(sexfiles_id));
                serial_println!("[kernel.cap.storage.spindle] spindle->sexfiles slot={}", sex_pdx::SLOT_STORAGE);
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_STORAGE target={} ok=1 optional=1]",
                    spindle_id, sexfiles_id
                );
            } else {
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_STORAGE target=missing ok=0 optional=1 reason=sexfiles_absent]",
                    spindle_id
                );
            }
            if sexbell_id != 0 {
                pd.grant_capability(sex_pdx::SLOT_BELL, CapabilityData::Domain(sexbell_id));
                serial_println!("[kernel.cap.bell.spindle] spindle->sexbell slot={}", sex_pdx::SLOT_BELL);
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_BELL target={} ok=1 optional=1]",
                    spindle_id, sexbell_id
                );
            } else {
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_BELL target=missing ok=0 optional=1 reason=sexbell_absent]",
                    spindle_id
                );
            }
            if linen_id != 0 {
                pd.grant_capability(sex_pdx::SLOT_LINEN, CapabilityData::Domain(linen_id));
                serial_println!("[kernel.cap.linen.spindle] spindle->linen slot={}", sex_pdx::SLOT_LINEN);
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_LINEN target={} ok=1 optional=1]",
                    spindle_id, linen_id
                );
            } else {
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_LINEN target=missing ok=0 optional=1 reason=linen_absent]",
                    spindle_id
                );
            }
            if silkshell_id != 0 {
                pd.grant_capability(sex_pdx::SLOT_SHELL, CapabilityData::Domain(silkshell_id));
                serial_println!("[kernel.cap.shell.spindle] spindle->silkshell slot={}", sex_pdx::SLOT_SHELL);
                serial_println!(
                    "[bootgraph.cap.grant from=kernel to={} slot=SLOT_SHELL target={} ok=1 optional=1]",
                    spindle_id, silkshell_id
                );
            }
            serial_println!("[kernel.cap.spindle] storage={} bell={} linen={} shell={}",
                sexfiles_id != 0, sexbell_id != 0, linen_id != 0, silkshell_id != 0);
        } else {
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_STORAGE target={} ok=0 optional=1 reason=pd_missing]",
                spindle_id, sexfiles_id
            );
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_BELL target={} ok=0 optional=1 reason=pd_missing]",
                spindle_id, sexbell_id
            );
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_LINEN target={} ok=0 optional=1 reason=pd_missing]",
                spindle_id, linen_id
            );
        }
    } else {
        serial_println!(
            "[bootgraph.cap.grant from=kernel to=missing slot=SLOT_STORAGE target={} ok=0 optional=1 reason=spindle_absent]",
            sexfiles_id
        );
    }

    // SexFiles → SexDrive block/DMA route: grant SLOT_BLOCK so DiskFS
    // can send DmaCall messages to the block device server.
    if sexfiles_id != 0 && sexdrive_id != 0 {
        use crate::ipc::DOMAIN_REGISTRY;
        use crate::capability::CapabilityData;
        if let Some(pd) = DOMAIN_REGISTRY.get(sexfiles_id) {
            pd.grant_capability(sex_pdx::SLOT_BLOCK, CapabilityData::Domain(sexdrive_id));
            serial_println!("[kernel.cap.block] sexfiles->sexdrive slot={}", sex_pdx::SLOT_BLOCK);
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_BLOCK target={} ok=1 optional=1]",
                sexfiles_id, sexdrive_id
            );
        } else {
            serial_println!(
                "[bootgraph.cap.grant from=kernel to={} slot=SLOT_BLOCK target={} ok=0 optional=1 reason=pd_missing]",
                sexfiles_id, sexdrive_id
            );
        }
    } else {
        serial_println!(
            "[bootgraph.cap.grant from=kernel to={} slot=SLOT_BLOCK target={} ok=0 optional=1 reason=sexfiles_or_sexdrive_absent]",
            sexfiles_id, sexdrive_id
        );
    }
    if kaleido_id != 0 && sexnet_id != 0 {
        use crate::ipc::DOMAIN_REGISTRY;
        use crate::capability::CapabilityData;
        serial_println!(
            "[slot.net.grant.browser.begin] target=kaleidoscope slot=18 pd={}",
            kaleido_domain_id
        );
        if let Some(pd) = DOMAIN_REGISTRY.get(kaleido_id) {
            pd.grant_capability(sex_pdx::SLOT_NET, CapabilityData::Domain(sexnet_id));
            serial_println!(
                "[slot.net.grant.browser.ok] slot=18 kaleido_pd={}",
                kaleido_domain_id
            );
        }
    }
    serial_println!("[kernel.sexnet.passive] spawned={} id={} slot_net_grant=0 browser_network=0",
        if sexnet_id != 0 { 1u64 } else { 0u64 }, sexnet_id as u64);
    serial_println!("[bootgraph.phase25.complete]");
    crate::boot_network::init(sexnet_id);

    // Hand framebuffer to sexdisplay: Limine fb.address is ALREADY VIRTUAL.
    if sexdisp_id != 0 {
        use crate::ipc::DOMAIN_REGISTRY;
        use crate::ipc::messages::MessageType;

        if let Some(fb_res) = crate::FB_REQUEST.response() {
            if let Some(fb) = fb_res.framebuffers().iter().next() {
                // Display's private frames are packed; the scanout may pad its rows.
                assert!(fb.pitch >= fb.width * 4 && fb.pitch % 4 == 0, "BOOT_DISPLAY_FRAME_PITCH");
                let layout = sex_pdx::display_frames::Layout::new(fb.width, fb.height, fb.width * 4)
                    .expect("BOOT_DISPLAY_FRAME_GEOMETRY");
                assert_eq!(fb.bpp, 32, "BOOT_DISPLAY_FRAME_FORMAT");
                let fb_addr = fb.address() as u64;
                let fb_size = fb.pitch * fb.height;
                let physical = sex_pdx::display_boot::Physical::from_edid(fb.edid(), fb.width as u32, fb.height as u32);
                report_display_modes(fb_res, fb.width, fb.height, physical);
                crate::boot_memory::init_display_frames(sexdisp_id, layout);
                crate::boot_display::init(crate::boot_display_state::Info{display:sexdisp_id,shell:silkshell_id,input:sexinput_id,address:fb_addr,width:fb.width as u32,height:fb.height as u32,pitch:fb.pitch as u32,physical:physical.pack()});

                // Remap FB pages USER_ACCESSIBLE — Ring-3 sexdisplay can't write without this.
                {
                    // Use manual page-table walk that handles huge pages (2MiB, 1GiB).
                    // The old mapper.update_flags(Page<Size4KiB>) silently returns
                    // Err(ParentEntryHugePage) when the framebuffer is mapped with huge pages,
                    // leaving USER_ACCESSIBLE unset and causing #GP from ring 3.
                    let pkey = DOMAIN_REGISTRY.get(sexdisp_id).expect("BOOT_DISPLAY_DOMAIN").pku_key;
                    let start = fb_addr & !0xFFF;
                    let end = fb_addr.checked_add(fb_size).and_then(|end| end.checked_add(4095))
                        .map(|end| end & !0xFFF).expect("BOOT_DISPLAY_FRAMEBUFFER_EXTENT");
                    for va in (start..end).step_by(4096) {
                        unsafe { crate::pku::set_page_user_accessible(va, pkey); }
                    }
                    serial_println!("init: FB remapped USER_ACCESSIBLE ({:#x}, {} bytes) key={}",
                        fb_addr, fb_size, pkey);
                }

                let msg = MessageType::DisplayPrimaryFramebuffer {
                    virt_addr: fb_addr,
                    width:  fb.width  as u32,
                    height: fb.height as u32,
                    pitch:  (fb.pitch / 4) as u32,
                };

                if let Some(pd) = DOMAIN_REGISTRY.get(sexdisp_id) {
                    unsafe { let _ = (*pd.message_ring).enqueue(msg); }
                    serial_println!("init: FB handed to sexdisplay ({}x{} @ {:#x})", fb.width, fb.height, fb_addr);
                }
            }
        }
    }

    // Final BOOT_DAG memory edges precede sealing and every task admission.
    crate::boot_memory::init(sexdrive_id, sexdisp_id, sexusb_id, sexnet_id, sexfiles_id);
    crate::boot_memory::init_documents(sexfiles_id, sex_pdx::app_catalog::QUIL_TARGETS);
    assert_eq!((silkshell_id, sexdisp_id), (sex_pdx::uiir::SHELL_PD, sex_pdx::uiir::DISPLAY_PD), "BOOT_UIIR_DOMAINS");
    crate::boot_memory::init_uiir(silkshell_id, sexdisp_id);
    crate::boot_memory::init_text_regions(sexdisp_id);
    crate::boot_memory::init_pixel_regions(sexdisp_id);
    crate::boot_memory::init_sound_rings(sexbell_id);
    crate::boot_memory::init_program_image(sexfiles_id);
    crate::boot_memory::init_fs_regions(sexfiles_id);
    crate::boot_memory::init_boot_image(sexfiles_id);
    crate::boot_memory::init_web_regions(sexnet_id);
    crate::program_tenancy::claim_regions();
    // The network service parses the internet, so it is the one most likely to fall over.
    crate::service_restart::capture(sexnet_id, 13);
    // Files holds the disk, but its filesystem commits atomically, so a fresh run remounts it.
    crate::service_restart::capture(sexfiles_id, 11);
    // Input keeps nothing but what is in flight; a new run picks up the next key.
    crate::service_restart::capture(sexinput_id, 4);
    // Bell's sound state is rebuilt by its own start; its paging check runs once.
    crate::service_restart::capture(sexbell_id, 10);
    // Display: the screen comes back from BOOT's handoff; Shell shows its windows again.
    crate::service_restart::capture(sexdisp_id, 1);
    // Shell: a new run rebuilds its windows from the app catalog.
    crate::service_restart::capture(silkshell_id, 3);
    crate::program_records::init(silkshell_id);
    {
        let shell=crate::ipc::DOMAIN_REGISTRY.get(silkshell_id).expect("BOOT_SHELL_MISSING");
        assert!(shell.find_capability(sex_pdx::program_store::SHELL_ROUTE_SLOT as u32).is_none(),"BOOT_PROGRAM_ROUTE_SLOT");
        shell.grant_capability(sex_pdx::program_store::SHELL_ROUTE_SLOT,crate::capability::CapabilityData::Domain(sexfiles_id));
    }
    assert!(sext_id != 0 && kaleido_id != 0 && sexbell_id != 0, "BOOT_PAGING_DOMAINS_MISSING");
    // Replace the client's initial self-management edge at static genesis.
    // The capability graph remains sealed throughout runtime fault recovery.
    for client in [kaleido_id, sexbell_id] {
        crate::ipc::DOMAIN_REGISTRY.get(client).expect("BOOT_PAGING_CLIENT_MISSING")
            .grant_capability(sex_pdx::SLOT_SEXT, crate::capability::CapabilityData::Domain(sext_id));
        serial_println!("[bootgraph.cap.grant from=kernel to={} slot=SLOT_SEXT target={} ok=1]", client, sext_id);
    }
    // Kaleidoscope waits for Bell's normal service reply after Bell's paging
    // exercise. This is a static service edge, sealed with every other edge.
    crate::ipc::DOMAIN_REGISTRY.get(kaleido_id).expect("BOOT_PAGING_CLIENT_MISSING")
        .grant_capability(sex_pdx::SLOT_BELL, crate::capability::CapabilityData::Domain(sexbell_id));
    serial_println!("[bootgraph.cap.grant from=kernel to={} slot=SLOT_BELL target={} ok=1]", kaleido_id, sexbell_id);
    // Native app service routes are immutable genesis edges, including cleanup proof leases.
    for (slot,target) in [(sex_pdx::SLOT_DISPLAY,sexdisp_id),(sex_pdx::SLOT_SHELL,silkshell_id),(sex_pdx::SLOT_STORAGE,sexfiles_id)] {
        crate::ipc::DOMAIN_REGISTRY.get(kaleido_id).expect("BOOT_APP_MISSING")
            .grant_capability(slot,crate::capability::CapabilityData::Domain(target));
        serial_println!("[boot.app.route] target={} slot={} service={} runtime_mint=0",kaleido_id,slot,target);
    }
    crate::boot_memory::init_paging(sext_id, [kaleido_id, sexbell_id]);
    // Shell close requests use fixed genesis routes; no runtime lookup/mint.
    for (slot,target) in sex_pdx::app_lifecycle::CLOSE_SLOTS.into_iter().zip([quil_id,spindle_id,kaleido_id]) {
        let shell=crate::ipc::DOMAIN_REGISTRY.get(silkshell_id).expect("BOOT_SHELL_MISSING");
        assert!(shell.find_capability(slot as u32).is_none(),"BOOT_CLOSE_SLOT_OCCUPIED");
        shell.grant_capability(slot as u64,crate::capability::CapabilityData::Domain(target));
        serial_println!("[boot.app.close.route] holder={} target={} slot={} runtime_mint=0",silkshell_id,target,slot);
    }
    crate::app_resources::init(sext_id, silkshell_id,
        &sex_pdx::app_catalog::CONTENT,
        [silkshell_id,sexdisp_id,sexfiles_id,sexnet_id,sexbell_id]);
    // Each successor's service edges and Shell's close/route edges to it.
    let shell=crate::ipc::DOMAIN_REGISTRY.get(silkshell_id).unwrap();
    for incarnation in sex_pdx::app_catalog::INCARNATIONS.iter().filter(|i|i.ordinal!=0||i.app==sex_pdx::app_catalog::App::Program) {
        let pd=crate::ipc::DOMAIN_REGISTRY.get(incarnation.target).unwrap();
        let spindle=incarnation.app==sex_pdx::app_catalog::App::Spindle;
        for (slot,service) in [(sex_pdx::SLOT_DISPLAY,sexdisp_id),(sex_pdx::SLOT_STORAGE,sexfiles_id),(sex_pdx::SLOT_SHELL,silkshell_id),(sex_pdx::SLOT_LINEN,linen_id),(sex_pdx::SLOT_BELL,sexbell_id)].into_iter().take(if spindle{5}else{3}){
            // The storage slot may hold the image's own VFS node; nothing else may collide.
            match pd.find_capability(slot as u32).map(|c|c.data) {
                None=>{},
                Some(crate::capability::CapabilityData::Node(_)) if slot==sex_pdx::SLOT_STORAGE=>{},
                _=>panic!("BOOT_SUCCESSOR_ROUTE_COLLISION"),
            }
            pd.grant_capability(slot,crate::capability::CapabilityData::Domain(service));
        }
        if incarnation.app==sex_pdx::app_catalog::App::Browser {
            // Normal Browser startup and typed source protocols use Net. Bell
            // supports the existing configured cleanup exercise; successors do
            // not inherit the original's demand-paging grant or private leaves.
            grant_browser_successor_route(pd,sex_pdx::SLOT_NET,sexnet_id);
            grant_browser_successor_route(pd,sex_pdx::SLOT_BELL,sexbell_id);
        }
        for slot in [incarnation.close_slot,incarnation.route_slot]{
            assert!(shell.find_capability(slot as u32).is_none(),"BOOT_SUCCESSOR_SHELL_COLLISION");
            shell.grant_capability(slot,crate::capability::CapabilityData::Domain(incarnation.target));
        }
    }
    for id in sex_pdx::app_catalog::TARGETS{
        let pd=crate::ipc::DOMAIN_REGISTRY.get(id).unwrap();let task=pd.main_task.load(core::sync::atomic::Ordering::Acquire);
        assert!(pd.find_capability(sex_pdx::app_lifecycle::SELF_CATALOG_SLOT as u32).is_none(),"BOOT_SELF_CATALOG_COLLISION");
        pd.grant_capability(sex_pdx::app_lifecycle::SELF_CATALOG_SLOT as u64,crate::capability::CapabilityData::AppLifecycle(
            crate::app_resource_state::LifecycleGrant{target_pd:id,task_id:unsafe{(*task).id},holder_pd:id,rights:crate::app_resource_state::QUERY,ack_bit:0}));
    }
    // All plans describe actual catalog Browsers. Preflight all slots before writing.
    assert_eq!(crate::ipc::BOOT_PHASE.load(core::sync::atomic::Ordering::Acquire),crate::ipc::BootPhase::RegistryBuild as u8,"BOOT_SOURCE_GENESIS_PHASE");
    assert_eq!(sexnet_id,13,"BOOT_SOURCE_HOLDER");
    let seed_holder=crate::ipc::DOMAIN_REGISTRY.get(sexnet_id).expect("BOOT_SEED_HOLDER");
    assert_eq!(sex_pdx::browser_sources::PLANS.len(),sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Browser),"BOOT_SOURCE_PLAN_COVERAGE");
    for(index,plan)in sex_pdx::browser_sources::PLANS.iter().copied().enumerate(){
        let _=browser_source_grants(seed_holder,plan);
        assert!(!sex_pdx::browser_sources::PLANS[..index].iter().any(|p|p.target()==plan.target()||p.surface()==plan.surface()),"BOOT_SOURCE_PLAN_DUPLICATE");
        let slots=[plan.entropy_slot(),plan.clock_slot(),plan.trust_slot()];
        for(at,slot)in slots.into_iter().enumerate(){
            let decoded=crate::capability_slot::decode_capability_slot(slot).expect("BOOT_SOURCE_SLOT_WIDTH");
            assert!(!slots[..at].contains(&slot),"BOOT_SOURCE_SLOT_DUPLICATE");
            assert!(!sex_pdx::browser_sources::PLANS[..index].iter().any(|p|[p.entropy_slot(),p.clock_slot(),p.trust_slot()].contains(&slot)),"BOOT_SOURCE_PLAN_SLOT_COLLISION");
            assert!(seed_holder.find_capability(decoded).is_none(),"BOOT_SOURCE_SLOT_OCCUPIED");
        }
    }
    for plan in sex_pdx::browser_sources::PLANS.iter().copied(){
        let(seed_grant,clock_grant,trust_grant)=browser_source_grants(seed_holder,plan);
        seed_holder.grant_capability(plan.entropy_slot(),crate::capability::CapabilityData::EntropySource(seed_grant));
        seed_holder.grant_capability(plan.clock_slot(),crate::capability::CapabilityData::ClockSource(clock_grant));
        seed_holder.grant_capability(plan.trust_slot(),crate::capability::CapabilityData::TrustPolicy(trust_grant));
        verify_browser_source_grants(seed_holder,plan);
        serial_println!("[boot.entropy.source] holder={} task={} slot={} consumer={} consumer_task={} source=rdseed attempts={} runtime_mint=0",sexnet_id,seed_grant.holder.task,plan.entropy_slot(),plan.target(),seed_grant.consumer.task,sex_pdx::entropy_source::ATTEMPTS);
        serial_println!("[boot.clock.source] holder={} task={} slot={} consumer={} consumer_task={} source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0",sexnet_id,clock_grant.holder.task,plan.clock_slot(),plan.target(),clock_grant.consumer.task);
    }
    // Independent, exact BOOT source for the status-bar publisher. Its bounded
    // read mechanism supplies explicit snapshots; neither timer cadence nor
    // renderer fallback confers civil-time authority.
    // Power: Shell may ask to shut down or restart, Files flushes first, the bar reads sensors.
    for (holder,rights,role) in [(silkshell_id,sex_pdx::power::REQUEST|sex_pdx::power::SENSE,"shell"),
        (sexfiles_id,sex_pdx::power::FLUSH,"files"),(silkbar_id,sex_pdx::power::SENSE,"bar")] {
        let Some(pd)=(holder!=0).then(||crate::ipc::DOMAIN_REGISTRY.get(holder)).flatten() else {continue};
        assert!(pd.find_capability(sex_pdx::power::SLOT as u32).is_none(),"BOOT_POWER_SLOT_COLLISION");
        pd.grant_capability(sex_pdx::power::SLOT,crate::capability::CapabilityData::Power(sex_pdx::power::Grant{holder_pd:holder,rights}));
        if rights&sex_pdx::power::FLUSH!=0 {crate::hal::acpi::set_flusher(holder);}
        serial_println!("[boot.power.grant] holder={} role={} slot={} rights={:#x} runtime_mint=0",holder,role,sex_pdx::power::SLOT,rights);
    }
    let bar_holder=crate::ipc::DOMAIN_REGISTRY.get(silkbar_id).expect("BOOT_BAR_CLOCK_HOLDER");
    let bar_task=bar_holder.main_task.load(core::sync::atomic::Ordering::Acquire);
    assert!(!bar_task.is_null(),"BOOT_BAR_CLOCK_TASK");
    let bar_identity=sex_pdx::clock_source::Identity{pd:silkbar_id,task:unsafe{(*bar_task).id}};
    assert_eq!(bar_identity,sex_pdx::clock_source::STATUS_BAR,"BOOT_BAR_CLOCK_ROLE");
    assert!(bar_holder.find_capability(sex_pdx::clock_source::SLOT as u32).is_none(),"BOOT_BAR_CLOCK_SLOT_COLLISION");
    bar_holder.grant_capability(sex_pdx::clock_source::SLOT,crate::capability::CapabilityData::ClockSource(
        sex_pdx::clock_source::Grant{holder:bar_identity,consumer:bar_identity,source:sex_pdx::clock_source::CMOS_UTC}));
    serial_println!("[boot.silkbar.clock.source] holder={} task={} slot={} consumer={} consumer_task={} source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0",bar_identity.pd,bar_identity.task,sex_pdx::clock_source::SLOT,bar_identity.pd,bar_identity.task);
    for plan in sex_pdx::browser_sources::PLANS.iter().copied(){
        let(_,_,trust_grant)=browser_source_grants(seed_holder,plan);
        serial_println!("[boot.trust.policy] holder={} task={} slot={} consumer={} consumer_task={} profile={} root_bytes={} runtime_mint=0",sexnet_id,trust_grant.holder.task,plan.trust_slot(),plan.target(),trust_grant.consumer.task,trust_grant.profile,sex_pdx::trust_config::ROOT_DER.len());
        assert_eq!(sex_pdx::network_config::HOST_NAME,sex_pdx::trust_config::EXPECTED_NAME.as_bytes(),"BOOT_TLS_EXPECTED_NAME");
        assert!(sex_pdx::network_config::Config::CONTROLLED.tls_endpoint().is_some(),"BOOT_TLS_ENDPOINT_PROFILE");
        serial_println!("[boot.network.tls] holder={} task={} consumer={} consumer_task={} profile=1 peer_port=46080 origin=immutable_boot_profile runtime_mint=0",sexnet_id,trust_grant.holder.task,plan.target(),trust_grant.consumer.task);
    }
    // Two finite Spindle incarnations share only the fixed peer service edge.
    // This is BOOT genesis; neither app receives NIC metadata or config rights.
    assert!(sexnet_id!=0,"BOOT_SPINDLE_NETWORK_SERVICE");
    for target in sex_pdx::app_catalog::INCARNATIONS.iter().filter(|i|i.app==sex_pdx::app_catalog::App::Spindle).map(|i|i.target) {
        let pd=crate::ipc::DOMAIN_REGISTRY.get(target).expect("BOOT_SPINDLE_NETWORK_SUBJECT");
        assert!(pd.find_capability(sex_pdx::SLOT_NET as u32).is_none(),"BOOT_SPINDLE_NETWORK_COLLISION");
        pd.grant_capability(sex_pdx::SLOT_NET,crate::capability::CapabilityData::Domain(sexnet_id));
        let task=pd.main_task.load(core::sync::atomic::Ordering::Acquire);
        serial_println!("[boot.spindle.net.route] target={} task={} slot=18 service={} fixed_peer=1 nic_metadata=0 runtime_mint=0",target,unsafe{(*task).id},sexnet_id);
    }
    // Program rows hold the same fixed peer edge; the network service serves only the runs
    // Shell binds, and Shell holds a route there for that alone.
    for row in sex_pdx::app_catalog::INCARNATIONS.iter().filter(|i|i.app==sex_pdx::app_catalog::App::Program) {
        let pd=crate::ipc::DOMAIN_REGISTRY.get(row.target).expect("BOOT_PROGRAM_NETWORK_SUBJECT");
        assert!(pd.find_capability(sex_pdx::SLOT_NET as u32).is_none(),"BOOT_PROGRAM_NETWORK_COLLISION");
        pd.grant_capability(sex_pdx::SLOT_NET,crate::capability::CapabilityData::Domain(sexnet_id));
        // Bell plays a program's sound from that program's own ring; the edge carries
        // only "my ring has sound" and Bell's own tone and notice calls.
        if sexbell_id!=0 {
            assert!(pd.find_capability(sex_pdx::SLOT_BELL as u32).is_none(),"BOOT_PROGRAM_BELL_COLLISION");
            pd.grant_capability(sex_pdx::SLOT_BELL,crate::capability::CapabilityData::Domain(sexbell_id));
        }
    }
    {
        let shell=crate::ipc::DOMAIN_REGISTRY.get(silkshell_id).expect("BOOT_SHELL_NETWORK");
        assert!(shell.find_capability(sex_pdx::web::SHELL_SLOT as u32).is_none(),"BOOT_SHELL_NETWORK_COLLISION");
        shell.grant_capability(sex_pdx::web::SHELL_SLOT,crate::capability::CapabilityData::Domain(sexnet_id));
    }
    // One-shot activation chain: each successor is bound to its exact predecessor.
    for incarnation in sex_pdx::app_catalog::INCARNATIONS.iter().filter(|i|i.ordinal!=0) {
        let predecessor=sex_pdx::app_catalog::predecessor(incarnation).expect("BOOT_SUCCESSOR_PREDECESSOR");
        crate::app_resources::reserve_successor(predecessor.target,incarnation.target,silkshell_id,incarnation.activation_slot.expect("BOOT_SUCCESSOR_ACTIVATION_SLOT"));
    }
    // Linen serves Spindle's existing async list/open/search route. A read-only
    // lifecycle observer fences queued requests after death without owning an ACK.
    let linen=crate::ipc::DOMAIN_REGISTRY.get(linen_id).expect("BOOT_LINEN_MISSING");
    let mut history_participants=[sex_pdx::history_recovery::Participant::EMPTY;sex_pdx::app_catalog::SPINDLE_COUNT];
    for(index,row)in sex_pdx::app_catalog::spindle_views().enumerate(){
        let domain=crate::ipc::DOMAIN_REGISTRY.get(row.target).expect("BOOT_HISTORY_DOMAIN");
        let main=domain.main_task.load(core::sync::atomic::Ordering::Acquire);
        assert!(!main.is_null(),"BOOT_HISTORY_MAIN_TASK");
        let task=unsafe{(*main).id};
        history_participants[index]=sex_pdx::history_recovery::Participant{target:row.target,task};
        assert!(linen.find_capability(row.lifecycle_slot as u32).is_none(),"BOOT_HISTORY_OBSERVER_COLLISION");
        linen.grant_capability(row.lifecycle_slot as u64,crate::capability::CapabilityData::AppLifecycle(
            crate::app_resource_state::LifecycleGrant{target_pd:row.target,task_id:task,holder_pd:linen_id,
                rights:crate::app_resource_state::QUERY,ack_bit:0}));
        serial_println!("[boot.app.observer] holder={} target={} task={} slot={} query=1 ack=0 retire=0",linen_id,row.target,task,row.lifecycle_slot);
    }
    let files=crate::ipc::DOMAIN_REGISTRY.get(sexfiles_id).expect("BOOT_HISTORY_FILES");
    assert!(files.find_capability(sex_pdx::history_recovery::SLOT as u32).is_none());
    let history_grant=sex_pdx::history_recovery::Grant{holder:sexfiles_id,
        logical:sex_pdx::app_catalog::SPINDLE_TARGETS[0],participants:history_participants,
        base:sex_pdx::SPINDLE_HISTORY_BASE_LBA,sectors:sex_pdx::SPINDLE_HISTORY_SECTORS};
    assert!(history_grant.valid(),"BOOT_HISTORY_ROSTER_INVALID");
    files.grant_capability(sex_pdx::history_recovery::SLOT as u64,crate::capability::CapabilityData::HistoryRecovery(history_grant));
    // Compare every published field with the capability actually inserted.
    for kind in 0..history_participants.len()as u64+2 {
        assert_eq!(crate::history_recovery::query(&files,sex_pdx::history_recovery::SLOT as u64,kind),history_grant.query(sexfiles_id,kind),"BOOT_HISTORY_HELD_QUERY_MISMATCH");
    }
    assert!(core::mem::size_of::<crate::capability::CapabilityData>()<=128,"BOOT_HISTORY_CAPABILITY_BUDGET");
    serial_println!("[boot.history.layout] grant_bytes={} capability_data_bytes={} capability_bytes={} participants={} bounded=1",core::mem::size_of::<sex_pdx::history_recovery::Grant>(),core::mem::size_of::<crate::capability::CapabilityData>(),core::mem::size_of::<crate::capability::Capability>(),history_participants.len());
    serial_println!("[boot.history.recovery] holder={} slot={} logical={} participants={} version={} base={} sectors={} held_query_verified=1 runtime_mint=0",sexfiles_id,sex_pdx::history_recovery::SLOT,history_grant.logical,history_participants.len(),sex_pdx::history_recovery::VERSION,history_grant.base,history_grant.sectors);
    for(index,participant)in history_participants.iter().enumerate(){
        serial_println!("[boot.history.participant] index={} target={} task={} holder={} immutable=1 runtime_mint=0",index,participant.target,participant.task,sexfiles_id);
    }
    crate::app_resources::audit_boot_catalog(silkshell_id);
    crate::scheduler::seal_self_wait_inventory();
    for plan in sex_pdx::browser_sources::PLANS.iter().copied(){
        let info=verify_browser_source_grants(seed_holder,plan);
        serial_println!("[boot.browser.sources.readback] holder={} target={} entropy={} clock={} trust={} late_before_seal=1 typed_info=1 runtime_context=0 trust_profile={} trust_root_bytes={} trust_version={}",sexnet_id,plan.target(),plan.entropy_slot(),plan.clock_slot(),plan.trust_slot(),info.profile,info.root_bytes,info.version);
    }
    // Every device lease and DMA pool exists now: give each driver its DMA domain.
    crate::iommu::init_boot();
    crate::boot_memory::seal_all();

    // Snapshot the actual kernel heap after all reserved images/grants exist.
    // Drop its allocator guard before formatting/serial I/O. This is heap
    // accounting at this BOOT point, not a physical free-frame watermark.
    let (heap_bytes,heap_used,heap_free)={
        let heap=crate::ALLOCATOR.lock();
        (heap.size(),heap.used(),heap.free())
    };
    assert!(heap_bytes>0 && heap_used.checked_add(heap_free)==Some(heap_bytes),"BOOT_HEAP_BUDGET");
    serial_println!("[boot.heap.budget] bytes={} used={} free={} apps={} point=after_grant_seal allocator_lock_released=1 physical_free_frames=unmeasured",heap_bytes,heap_used,heap_free,apps::COUNT);

    serial_println!("init: Revoking kernel write access...");

    // Enqueue all registered PD tasks onto scheduler runqueue
    for pd_id in 1..crate::program_records::BASE {
        if let Some(pd) = crate::ipc::DOMAIN_REGISTRY.get(pd_id) {
            if !crate::app_resources::activation_permitted(pd_id) {continue;}
            let task_ptr = pd.main_task.load(core::sync::atomic::Ordering::Acquire);
            if !task_ptr.is_null() {
                let pd_name = match pd_id {
                    1 => "sexdisplay",
                    2 => "sexdrive",
                    3 => "silk-shell",
                    4 => "sexinput",
                    5 => "sexusb",
                    6 => "silkbar",
                    7 => "linen",
                    8 => "sexstore",
                    9 => "quil",
                    10 => "sexbell",
                    11 => "sexfiles",
                    12 => "spindle",
                    13 => "sexnet",
                    14 => "sext",
                    15 => "kaleidoscope",
                    _ => "unknown",
                };
                let entry = unsafe { (*task_ptr).context.rip };
                serial_println!(
                    "[bootgraph.boot.handoff] target={} id={} entry={:#x}",
                    pd_name, pd_id, entry
                );
                assert!(crate::scheduler::SCHEDULERS[0].activate_task(task_ptr),
                    "BOOT_TASK_ALREADY_ADMITTED_OR_UNCONSTRUCTED");
                // SCHEDULER_TICK_PD8_PF_FLAKE_V1 phase-2: task ptr + pd_ptr
                // field address, for hardware-watchpoint corruption capture.
                serial_println!(
                    "scheduler.enqueue pd_id={} task={:#x} pdptr_field={:#x}",
                    pd_id,
                    task_ptr as u64,
                    unsafe { core::ptr::addr_of!((*task_ptr).context.pd_ptr) as u64 }
                );
            }
        }
    }

    unsafe {
        crate::ipc::BOOT_CONTROLLER.advance(crate::ipc::BootPhase::RegistryFrozen);
        crate::ipc::BOOT_CONTROLLER.advance(crate::ipc::BootPhase::SchedulerArmed);
        crate::ipc::BOOT_CONTROLLER.advance(crate::ipc::BootPhase::SchedulerRunning);
    }

    serial_println!("init: Ready for Scheduler.");
}

fn pdx_spawn(name: &str, domain_id: u8) -> Result<u32, &'static str> {
    use crate::pd::create::create_protection_domain;
    create_protection_domain(name, None, domain_id)
}

pub unsafe fn jump_to_userland(pd_id: u32, entry: u64, pkru: u32, pku_key: u8) -> ! {
    use crate::gdt;
    let selectors = gdt::get_selectors();
    
    // User Code Segment (0x28) with RPL 3 = 0x2B
    let user_cs = (selectors.user_cs.0 | 3) as u64;
    // User Data Segment (0x20) with RPL 3 = 0x23
    let user_ss = (selectors.user_ss.0 | 3) as u64;
    
    let rflags: u64 = 0x3202;
    let stack_top = 0x_7000_0000_0000 + (pku_key as u64 * 0x100_0000) + (64 * 1024);

    // crate::core_local::CoreLocal::get().set_pd(pd_id); // Deprecated, jump_to_userland is dead code

    core::arch::asm!(
        "xor eax, eax", "xor ecx, ecx", "xor edx, edx", "wrpkru", // God Mode
        "push {ss}",
        "push {rsp_val}",
        "push {rflags}",
        "push {cs}",
        "push {rip}",
        "mov eax, {target_pkru:e}", "xor ecx, ecx", "xor edx, edx", "wrpkru", // Isolation
        "swapgs",
        "iretq",
        ss      = in(reg) user_ss,
        rsp_val = in(reg) (stack_top & !0xFu64) - 64,
        rflags  = in(reg) rflags,
        cs      = in(reg) user_cs,
        rip     = in(reg) entry,
        target_pkru = in(reg) pkru,
        options(noreturn)
    );
}

/// Log the mode the firmware set, the monitor's size and the scale chosen, and
/// the other modes on offer (the boot menu's native entry takes the monitor's
/// own preferred mode).
fn report_display_modes(fb_res:&limine::request::FramebufferResponse,width:u64,height:u64,physical:sex_pdx::display_boot::Physical){
    let modes=fb_res.framebuffers_rev1().and_then(|f|f.first().map(|fb|fb.modes())).unwrap_or(&[]);
    let largest=modes.iter().filter(|m|m.bpp==32).map(|m|(m.width,m.height)).max_by_key(|&(w,h)|w*h).unwrap_or((width,height));
    serial_println!("[display.mode] width={} height={} modes={} largest={}x{} width_mm={} height_mm={} scale={}",
        width,height,modes.len(),largest.0,largest.1,physical.width_mm,physical.height_mm,physical.scale);
}
