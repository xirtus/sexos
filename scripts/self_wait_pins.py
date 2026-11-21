# SPDX-License-Identifier: GPL-3.0-only
"""Reviewed source pins for check_self_wait.py, one per line.

Add a pin as one new line, with a blank line around it, anywhere among the
others: .gitattributes merges this file with merge=union, so two branches that
each add a pin keep both instead of conflicting. Never edit or reorder lines in
place; to retire a pin, delete its line. Order within a kind does not matter.

  ('timed', path, sha256 or wait-v1 fingerprint),  # why it was reviewed
      a reviewed timed-wait causal source (APPROVED_TIMED_WAIT_SOURCES)
  ('tree', '', sha256),  # why
      a reviewed whole kernel Rust tree digest, kept as a review receipt
"""
PINS = (
    ('timed', 'kernel/src/interrupts.rs', 'fn-v1:f6ef904984a5335ae8a8db7df93b5dc6da52805e9adc60d812aa3a76080d0e0b'),  # #456 hot-plug tick merged with master (x17 PR sweep; union of two reviewed versions), 2026-11-21

    ('timed', 'kernel/src/scheduler.rs', 'a23ecc7d73a7775b9d5b455435c99b58f3311e78ce2992788dc20604a91fccb2'),  # #468 cross-core handoff + pair affinity at 047e13f5 (admit takes the home core's slot, should_preempt honours it, idle has_queued sees it; wait functions unchanged), reviewed by WS4, 2026-11-20

    ('timed', 'kernel/src/scheduler.rs', '59a7fe67bfa8a5a0d230296bd391d5261c6fbaeba42c3c0c05bcd7db6f19265e'),  # #424 (kernel fault report, rearm purge) merged with master, 2026-11-20

    ('timed', 'kernel/src/interrupts.rs', 'fn-v1:395d7827fe51750aaa43f1cba2da266fc407f19ab9ac6dcad4e5f40e416121fe'),  # #424 (kernel fault report, rearm purge) merged with master, 2026-11-20

    ('timed', 'kernel/src/interrupts.rs', 'fn-v1:2b52c1f01b3c0987f571d39433d87d3dfb7fde6689853aa748d5af2fb227e02d'),  # kernel fault report: task, custody, pending_outgoing (WS2 soak fault), 2026-11-20

    ('timed', 'kernel/src/scheduler.rs', 'fn-v1:eacc16fd01fce7c181e70f2178153c355c3793337e24f7acd9c58afb90415611'),  # rearm purges stale handoff slots + resets IPC v2 state (WS2 soak fault), 2026-11-19

    ('timed', 'kernel/src/interrupts.rs', 'fn-v1:96213afb9ba13c65c30b8892bda0d19b66aaf74aeedfcfeede56e4187b00f438'),  # kernel page fault prints the words around RSP (WS2 soak fault), 2026-11-19

    ('timed', 'kernel/src/program_records.rs', 'fn-v1:50abfa8222c49ebe84edeabc79324366bf1e5519a09c19d63542f664c6215981'),  # wait functions only (fn-v1), master 41dd66f3, 2026-11-18

    ('timed', 'kernel/src/program_records.rs', '37561f94116d2c3c44b1bf88b8f22927ebd57eeec7630cd6e40a8d38150fb2a8'),

    ('timed', 'kernel/src/program_records.rs', 'e4f79ad810b5bba8aab649b5472154ebece0d4ae1c7b96ccf0cacf3b076de769'),  # record table allocated zeroed via CapabilityTable::new_boxed (no wait change), 2026-11-17

    ('timed', 'kernel/src/program_records.rs', 'd8dab16031564c8540b35ce5e45c28d1cbf297071e3ffc6d21e5aff7322f625b'),  # record code scanned for key instructions (stream 2), 2026-11-18

    ('timed', 'kernel/src/program_records.rs', '46adbb04a4ef0f0e057f3282c9c19fc1f8044a8a9f4d4e77b0a4c679dee03215'),  # stream 2 scan merged with the v2 capability table, 2026-11-18

    ('timed', 'kernel/src/program_records.rs', 'fb6aaf0bc13ba27eaf82ce9e9dfb3498125830ee711899aff368c0fdc25bb96f'),  # protection views merged with the stream 2 scan, 2026-11-19

    ('timed', 'kernel/src/pd/create.rs', 'fn-v1:69758987a8d686a5f4b046db74d530379e4eff061dbe018b3019886371056906'),  # wait functions only (fn-v1), master 41dd66f3, 2026-11-18

    ('timed', 'kernel/src/pd/create.rs', '3fff0e0483f767e2e8509515874d0b320496cea0671c85b21c4aa368bbdf9b1e'),

    ('timed', 'kernel/src/init.rs', 'fn-v1:fb2da8b1ece8812feee1502fa72b73deb6248639042d5ddd6e42422db14ed884'),  # wait functions only (fn-v1), master 41dd66f3, 2026-11-18

    ('timed', 'kernel/src/init.rs', 'fn-v1:5424d8c0a824856443014bb081b57f3177c07d452e9f2558827a645a3fd1299e'),  # optional sexgpu-nv spawn (domain 53) + NVIDIA keep/lease arguments in init(), no wait change, 2026-11-19

    ('timed', 'kernel/src/init.rs', '784b18c238608b124ed2f2e2cd8d1d0eeaf6933b7bf9291fbc07b230688f0f64'),

    ('timed', 'kernel/src/init.rs', 'a977d60363a05adb60083b90bc6d898913c4efcd81a80209d0b9a4946340e334'),

    ('timed', 'kernel/src/init.rs', '04012df3ba205083c64b904130b8cef0a43814099ad9109008c1d9957acd8550'),

    ('timed', 'kernel/src/init.rs', '26a66c2f676742ab23625962ad5d8d380be6a670e56d0a659d1c614305bbdd8e'),

    ('timed', 'kernel/src/init.rs', '905080c14b576c29a9f8dc0c90400bbd93d0b46723d4397fb851af5d317bd0e8'),

    ('timed', 'kernel/src/init.rs', '3ed435db8f36db8c12c6fbc2b39a041926891e0135920b0c7426cf2e7375dc0d'),

    ('timed', 'kernel/src/scheduler.rs', 'fn-v1:9cba8bed549187d4c2cfbc41d27a800f32aec67ba241ee5623534c60ebdc17e2'),  # IPC v2 wait spins in poll_for_partner (#426), 2026-11-19
    ('timed', 'kernel/src/scheduler.rs', 'fn-v1:d1ae84d320565569056b4890ee32b11af3b66bc34eba297a890387e8851f71ab'),

    ('timed', 'kernel/src/scheduler.rs', '1b10ca8d1b0d1eb3a092f0a61ed1ca7fe6a4b43c448bc24d560ead211940f8d9'),  # #429 merged with master (28ffcb59), reviewed by WS4, 2026-11-19

    ('timed', 'kernel/src/scheduler.rs', 'fn-v1:dea888ff27ad54e217839dc46b9a74e521d07b2be1c36256452fd7df937802f2'),  # master 586140ad: kernel stack guard page + boxed FPU area (no wait change), 2026-11-19

    ('timed', 'kernel/src/scheduler.rs', 'fn-v1:4670ed1968ba7f46d7097721884c9cf2c95de4bd69220374b07321f2f5eec750'),  # cross-core IPC polling + lean first look + bounded re-check (#429 / 80b89713), reviewed by WS4, 2026-11-18

    ('timed', 'kernel/src/scheduler.rs', 'fn-v1:8b328859b9fda2ff8e024287e6d020c11182c169ad993a0e786ad86033420fe6'),  # XSAVE/AVX heap save area + kernel stack canary (#427 / ed888ad5), reviewed by WS4, 2026-11-18

    ('timed', 'kernel/src/scheduler.rs', 'fn-v1:c92ee26a67f14c438179f32834b8b0a62f82d1dee42271d43863d4cc96f95a8f'),  # wait functions only (fn-v1), master 41dd66f3, 2026-11-18

    ('timed', 'kernel/src/scheduler.rs', '53aec880d2ea82474231d37aaae0862dcb8747185626ab6f4d79175a422ce8a8'),  # PKU gates: TSC_AUX on switch (P6.2a), 2026-11-19

    ('timed', 'kernel/src/scheduler.rs', 'fn-v1:458daaa0e25c5b534627e9cef612d5e71ba9f781e05b48689bee025a1fd4c70b'),  # master 53aec880: PKU gates TSC_AUX on switch (no wait change), 2026-11-19

    ('timed', 'kernel/src/scheduler.rs', 'fn-v1:3af49f2ffe4d2b73c67ec09f190aa8e46cf43a3a7b233f7d8a24532828203567'),  # #429 merged with master at 950c36ac (raw af1bffe7), wait change as reviewed at 28ffcb59, 2026-11-19

    ('timed', 'kernel/src/scheduler.rs', '586140ad4e39c4a7c9741196c75f0622d1742af04583ade9cadb1adde472b2ae'),  # kernel stack guard page + boxed FPU area (no wait change), 2026-11-18

    ('timed', 'kernel/src/scheduler.rs', '13063202325fb2dc39a4feb3d2aff8d24f2e842bbbc187ffe2cdfe6602ae02c1'),  # IPC v2 syscalls 105/106, 2026-11-18

    ('timed', 'kernel/src/scheduler.rs', '8956a2cebc2a3acd3be1b4f8fbab9bb967fcfd7304d66220854be44f61e64654'),  # IPC fast path (WS4), 2026-11-17

    ('timed', 'kernel/src/scheduler.rs', '0b75bce2639de6891a9ed4b7e3e0f871f9a5110bc23b9002b2912316e4ef7f64'),

    ('timed', 'kernel/src/scheduler.rs', '25c0bc835f30039f947f28bc79ed567e1c742e17ea3c9a8e0663b5f502c9d642'),  # per-task self-wait sampling (task_count), 2026-11-17

    ('timed', 'kernel/src/self_wait.rs', '0e7b60adbce245427e67bf1bb0ba20ddb929c304efbad13ce078fa79e43e4eaf'),

    ('timed', 'kernel/src/task_ownership.rs', '57e00d7269315b5810adb7d43bcc75e92a140b6fa02b1c2dd8c15533757564f6'),

    ('timed', 'kernel/src/smp.rs', '6f70b8da98e5fe7fb49bdb660bb44fb9df784b5245ed2ae71cf4517024afcbc4'),  # spare-core placement of USB, Bell, ACPI, Cufflinks over master's preferred-core order (table only, no wait change), 2026-11-21
    ('timed', 'kernel/src/smp.rs', '73db1b08514c89695981493296c4c94fbaeca8ea4022228aeb38b7c76dca833b'),  # USB, Bell, ACPI and Cufflinks start on spare cores (placement table only, no wait change), 2026-11-19

    ('timed', 'kernel/src/smp.rs', '261699be11866980d2a30b72cbb8d35ddccdc00aa1ee1cbceefadc716edc7d75'),  # protection views (tlb hook, rows spread), 2026-11-17

    ('timed', 'kernel/src/smp.rs', 'bd523573ed7c9576665af35b7f0d598ddccb5b9a44f21d5d85fb21a15d961325'),

    ('timed', 'kernel/src/smp.rs', 'b70ef0379662d01aa7559855d91b63c82d45146d10302f05e101717aa33aeac2'),  # preferred-core order (PROGRAM_ORDER, rank_program_cores), no wait change, 2026-11-20

    ('timed', 'kernel/src/interrupts.rs', 'fn-v1:235c80ce88b369d9d1da26b934e188c056b5e24330b1c242801deaf2e3854c86'),  # wait functions only (fn-v1), master 41dd66f3, 2026-11-18

    ('timed', 'kernel/src/interrupts.rs', 'c686d2ff054854086e2f27e872b1c0d0eafd3c9aa88636b716775b33288afd0d'),

    ('timed', 'kernel/src/interrupts.rs', 'dac8806ada26bbf32518d7a835d009c96bfd4787aa724a9fd29eefccc6fac62d'),

    ('timed', 'kernel/src/interrupts.rs', 'e79027cac805659384a0ab1ce0e651bed04727d37081c0e1a3e54d905910c43e'),  # entry hardening: NMI IST, NT/IOPL mask, cli + canonical RCX before SYSRET, swapgs lfence (2026-11-18)
    ('timed', 'kernel/src/interrupts.rs', 'fn-v1:9be7e0f042763f122e4881e3f91ce32256e08711eb9531cb73fac73865384dc4'),  # SMAP audit: a kernel fault on a user page is recorded and retried with AC; kernel faults keep PKRU (no wait change), 2026-11-20

    ('timed', 'kernel/src/interrupts.rs', 'fn-v1:c7ac0959a5dec36e2f31c4d10d81cca01f8a0903800114720f7a56a72237b250'),  # PCIe hot-plug slot look on the core-0 tick, 2026-11-19

    ('timed', 'kernel/src/syscalls/mod.rs', 'fn-v1:676eae4ad2ce1171bafcdd21eb7b0d61e5e387ff84e401eb926eecf173191fbb'),  # wait functions only (fn-v1), master 41dd66f3, 2026-11-18

    ('timed', 'kernel/src/syscalls/mod.rs', '5db4172f862dae745c2a5566327d5f0e0d66b7a7814beea7e49ad76651b3e577'),  # IPC v2 syscalls 105/106, 2026-11-18

    ('timed', 'kernel/src/syscalls/mod.rs', '055312eb92a25b451b946caef4f00d1999391840bb782a1697db72e1903b1c41'),

    ('timed', 'kernel/src/syscalls/mod.rs', '318f8602a3e053470c4f6f984fa818f928cae16253dcf9a9946edf1b114995ba'),  # IPC fast path (WS4), 2026-11-17

    ('timed', 'kernel/src/syscalls/mod.rs', 'aa6169383696167b349be8f37819f61fb896239b3ff7aa489f4ffac1a4e11524'),

    ('timed', 'kernel/src/syscalls/mod.rs', '425d2dc341b9dd2b5d03200a38a1ef85f0ed96de103e0524a5d51ba947500fa0'),

    ('timed', 'kernel/src/syscalls/mod.rs', 'd5b9e12bf9f40d9170ee7557343187559a2fe73909d90a251c3982c04a3f078f'),

    ('timed', 'kernel/src/program_workers.rs', 'fn-v1:83a85118b86748220c90c74a4050ea004db73bb95957bcbf1d7e447a3b0c2d24'),  # wait functions only (fn-v1), master 41dd66f3, 2026-11-18

    ('timed', 'kernel/src/program_workers.rs', 'c8ddc4a8002159a6814b651d32ef6fa7cb5f8b131b28e2ced837698bcc1c289b'),  # S9 row task cap (no wait change), 2026-11-19

    ('timed', 'kernel/src/program_workers.rs', '6180ef3fb5702e6bd63e1649b07df8ce39390c8fe69a6a9c93b2e199686dbe8e'),

    ('timed', 'kernel/src/program_workers.rs', '82050a7bd043822bd1593d798dbdf14381d5efba4122a71a63a0948287251f6a'),
    ('timed', 'kernel/src/program_workers.rs', 'fn-v1:2970f89125d00c2b9373f95433b8d983e6cb203d0edb3f643c4fa19eb6a30800'),  # SMAP: WAIT reads and EXIT writes the futex word through smap::user (no wait change), 2026-11-20

    ('timed', 'crates/sex-pdx/src/lib.rs', '8477d3326af90b744f72d300aa26af3b4b8ae7075527ca1b3aa622ea9ec5ccde'),

    ('timed', 'crates/sex-pdx/src/lib.rs', '9eb262eb2108d51f57ae9dc821e6e40a12a9e1003fbfac711422b246ec17fd44'),

    ('timed', 'crates/sex-pdx/src/lib.rs', 'f76238b37b9a35de5bdf5e7e62c95e6788e30e86411ef4ec77d8638b4b82ef05'),

    ('timed', 'crates/sex-pdx/src/lib.rs', 'f9dd629e5cdda6ba58fb130e988d33f0d3a88948278d2f414bf612dca02fc166'),

    ('timed', 'crates/sex-pdx/src/lib.rs', 'c2895d4991c2d48d099daee3e6090063b3288f77c0ad5e9a75201c903f95c984'),

    ('timed', 'crates/sex-pdx/src/lib.rs', 'ef6e695c7535ac077e8c4adef7ded6d79425210a1fd77005940be7f823cab40a'),

    ('timed', 'crates/sex-pdx/src/lib.rs', 'cee8e1525fa104c2c14f398b4eee80f4f63cd0f3f06602310cdf421dc15009c1'),

    ('timed', 'crates/sex-pdx/src/lib.rs', '1b7fe3f1752da8d10ea2f2ea6df5c387ff9ec9969a4287be64fb6e878f4a4d75'),

    ('timed', 'crates/sex-pdx/src/lib.rs', '562ccc2f5cf8cf755e68925f0a65a699fd9e968ac78a44bfe4c7dc84f464e7ab'),

    ('timed', 'crates/sex-pdx/src/lib.rs', 'f00f44669af45f707a4a24ec15f70321ed9ed04565007c9e1b093b42fdd39e96'),

    ('timed', 'crates/sex-pdx/src/lib.rs', '3766c55f455e02199154d9cdbe28c5fc93a454f91a2459fe04e2eafaf8abd24b'),

    ('timed', 'crates/sex-pdx/src/lib.rs', '74a5f4ccbd9aeeb574ff5c9d8f7008b486d89e214ef405ef589711dfda71904b'),

    ('timed', 'crates/sex-pdx/src/lib.rs', 'wait-v1:0f64f51bb2e99d980f283c55f402b56e3f5da5a1d22b814ac98b4b1867b8a3d2'),  # wait fingerprint (wait_fingerprint), 2026-11-16

    ('timed', 'crates/sex-pdx/src/lib.rs', 'wait-v1:aa1083137f9cf350f5a1fe54038943fa82596df2b4c8737d6e4f4fe2e4598946'),  # BLOCK_MAX_RUN 512 KiB + BLOCK_CMD_RUN (no wait change), 2026-11-17

    ('timed', 'servers/sexnet/src/main.rs', '00da2ed6f93bb473e3944a8a4bc7e38f887676083cfe8cf9195c821cba4fc725'),  # Reviewed restricted-provider intake/polling; the exact Net2 envelope and deadline owners are unchanged. See RESTRICTED_TIMER_SOURCE_REVIEW_2026_11_15.

    ('timed', 'servers/sexnet/src/main.rs', 'd7526adf2002e45afd5ad986808e65e7eee3d5f793f718e62f8893fef51ae10d'),

    ('timed', 'servers/sexnet/src/main.rs', 'fcf8299b6ce8b8acdd0bc005507036087d5569664c08d63da001dc9e694c7253'),

    ('timed', 'servers/sexnet/src/main.rs', '10fd38acaa45e76df8e7bd579e382b66365f46b944d2fc958104b8631476cb5b'),  # first 32 timed returns logged, 2026-11-17

    ('timed', 'servers/sexnet/src/main.rs', '361ad412d188a65e0e6ceebe7e71f518449aea32d466d276356ef10153261b12'),  # master merge of #460 with the NIC interrupt change (same line as PR #482), 2026-11-20

    ('timed', 'servers/sexnet/src/main.rs', 'd994aeb0af740e6157d92744656943261954b99c6828795723c9070d4625ff34'),  # wired NIC interrupt: 10-tick quiet sleep while armed, no stall after a full burst (two-tick envelope unchanged), 2026-11-19

    ('timed', 'servers/sexusb/src/main.rs', '3fb9341fda71262a023114c8115407949ce700c604c7c3327464e79a68af3a81'),

    ('timed', 'servers/sexusb/src/main.rs', 'adc83a02655343df1b83d99dedd252a7e9ee06164055a54420b5e9207b3dba06'),

    ('timed', 'servers/sexusb/src/main.rs', '129171cc0512a8097fe5535c36f8e76a1c358355432f9105f556fe2c454335ec'),
    ('timed', 'servers/sexnet/src/main.rs', 'fcc4ffc50a6302756285ae01c86fcd838852bfbdf4ac4a4b2198550bf2afe400'),  # OP_DEVICE_SLEEP from the kernel logged and skipped after the listen (no wait change), 2026-11-19
    ('timed', 'servers/sexnet/src/main.rs', '361ad412d188a65e0e6ceebe7e71f518449aea32d466d276356ef10153261b12'),  # #451 NIC interrupt sleep + #460 OP_DEVICE_SLEEP notice merged together (both reviewed, two-tick envelope unchanged), 2026-11-20
    ('timed', 'servers/sexnet/src/main.rs', '361ad412d188a65e0e6ceebe7e71f518449aea32d466d276356ef10153261b12'),  # master merge of the OP_DEVICE_SLEEP handler (#460) with the wired NIC interrupt wait (irq_wait, reviewed in its own PR); nothing new, 2026-11-20
    ('timed', 'servers/sexusb/src/main.rs', '2c15ba53f84180a5feb262de17783e3821be45c5f4941d9660835d9c9bb541a6'),  # sleep notice suspends idle ports (bounded link waits); quiet predicate unchanged, 2026-11-19

    ('timed', 'servers/sexusb/src/main.rs', '96d3ea5a52156559eaf7106a145ffe1b08445eb88798fe64645bf92da8abeaae'),  # USB audio class streaming (Qupid #392), 2026-11-19

    ('timed', 'servers/sexnet/src/main.rs', '361ad412d188a65e0e6ceebe7e71f518449aea32d466d276356ef10153261b12'),  # master's Net as of 2026-11-20 (unpinned on master), pinned by Qupid #392

    ('timed', 'servers/sexusb/src/main.rs', '09b8a45c44bf3af5b7941823186b2ad09655ddf6fa57ba60bfbf215cabc3c667'),  # USB audio streaming merged with sleep suspend (Qupid #392), 2026-11-20
    ('timed', 'servers/sexusb/src/main.rs', 'e78d6073dc41fe5b46b75e962e0ac3b5b9e7ab347bb9b95e5e21fed12a3f4e87'),  # Thunderbolt manager polled each pass, waits unchanged, 2026-11-19

    ('timed', 'servers/sexusb/src/main.rs', '09a173d46a7f99edd8c810029e767552230e343f4b9ece7d69e161fe18d7ba5a'),  # Thunderbolt unload/GO2SX on the sleep notice, waits unchanged, 2026-11-20

    ('timed', 'servers/sexusb/src/main.rs', '29fbd1f1d8d3cef6b20bb8920959aa1984650f57483afe0f20e787314c26576b'),  # USB audio merged with Thunderbolt (Qupid #392), 2026-11-20

    ('tree', '', '8ec4ce1789d6ddf9623378e848505d1b78ed2a01dcf61cd9cc3e30bb37e260c5'),  # master after #407 + #408 + #422 combined, 2026-11-18

    ('tree', '', 'dd0b57a2633e5dcf5bbfe653d883f3172ff44976bbb5d670af8b114a8d2c6fb2'),  # XSAVE + AVX (stream 5), 2026-11-18

    ('tree', '', '417835283472c60cfd1455bccc223876d2936126736b182b47c1e75f6ed56260'),  # Phase 1a dead kernel modules removed, 2026-11-18

    ('tree', '', '620ffd8bab9d6528e4842df3930d060d4ceacc071b7587db37604bd3ddaf7c6d'),  # IPC v2 objects (#406) + #410 merged on master, 2026-11-18

    ('tree', '', 'ad54b57094dec16de9007a317b15506afc52747967aa9d4fedd100a32b1d95dd'),  # IPC fast path (WS4), 2026-11-17

    ('tree', '', '91e41656109be5dd659d70a70718e0640f68372f35762c34d95a28260b807e92'),

    ('tree', '', 'dd69100da62b94d190510e266327094b2990443442b2a59326b4e60afd2792dc'),

    ('tree', '', '1e5f9a4228d4f0342d7d54c4cbc2cba37b9c59ba1923e0d82b1afa50a687dbcb'),

    ('tree', '', '42f85248058717dfcb4983c4209337b544347a2167abfe769fe30a86330c94b4'),

    ('tree', '', 'b84ac4b152e0920d553a0a5121b31ea516522801e09aac03a499a45637d83707'),

    ('tree', '', 'd39d8695743904116ed5e7bc98b47524ffd018d2b28c17ca16d5a1eddc56307c'),

    ('tree', '', 'fa1efe78d28850a66b795eb44f7ff544f85ac9033673335a02e8ae5fc54056eb'),

    ('tree', '', 'e93d67e07151c4c0f5eed240e0f3a27f9abb58074e26f52934eb8c36963278f7'),

    ('tree', '', 'a897c8b8c617cfef1ec3a7fad038f32f8530b232cf698f9ea896a89950d5a0ec'),  # per-task self-wait sampling, 2026-11-17

    ('tree', '', '95ef2223d462a85888843a0700105ff4538f044360df25138dde638253e06fb5'),  # WS2 kernel heap counters (SYS_STATS OP_KERNEL), unused slab.rs removed, 2026-11-17

    ('tree', '', '24a9360f565da793ccaa3f9dda02c0540af671c7d7f997ea81d9286a28ce9865'),  # entry hardening + security audit (stream 2), 2026-11-18

    ('tree', '', 'bdaa310d6ed632686c4284e34dcee8f2264ef16010014a6caac0013b998cf5bb'),  # BOOT loader refuses key instructions and W+X code (stream 2), 2026-11-18

    ('tree', '', '98e232964db1dbed07859aac5a285b9f80dbfdc87a7fe74adb0d23e7a0ca43ad'),  # stream 2 on top of the IPC fast path (#402), 2026-11-18

    ('tree', '', 'ddd84cbbe668235c1afc410fcaa0373b5388a992f71c1cdf89d39bb1c7173345'),  # W^X for every user mapping + record code scan (stream 2), 2026-11-18

    ('tree', '', '471f85c4ed8912eed8ae8b6ace5e1b6ccef5fc92af580498e3bdc0454c0b582f'),  # stream 2 merged with master (core_local page removed upstream), 2026-11-18

    ('tree', '', 'c620598cd6151890bb134924fcbb76dbe7ab8396503971b66f7e52b1c8251cbd'),  # Kernel plan phase 2: kernel/src/ipc_v2 (endpoint, reply, notification), not wired yet, 2026-11-17

    ('tree', '', '6653e237101288231f1336156721a6401df232d53786312b2ec222f5db2438b3'),  # stream 2 merged with ipc_v2 (stream 4), 2026-11-18

    ('tree', '', '55803e9cff54b5b46b50be5a697c71617e5461f48a37cc9f99168f1046ed2cfd'),  # stream 2 merged with the shared-image CI (#399), 2026-11-18

    ('tree', '', '6371383dfa90b1b27da40ad384b65ebd757ae90284fe6f0f1a083453fb6b1933'),  # authority v2 capability table (inline slots, generations), cheri.rs removed, 2026-11-17

    ('tree', '', 'ace8232c43ac9354d1997d8aed554d48ad8a39bf26507479a00cab0affaea3ae'),  # stream 2 merged with authority v2 and XSAVE (#407, #408), 2026-11-18

    ('tree', '', '22a70bcd8ad7b76d717a4484845ef6920d18fca47b892b0ae4f993a4953a0909'),  # IPC v2 syscalls 105/106 wired (kernel plan phase 2), 2026-11-18

    ('tree', '', 'c74db80e5ed8fa929a2a5253c80949ec3beffe467cfef29ac192dae43234902c'),  # stream 2 merged with IPC v2 syscalls (#418/#421/#422), 2026-11-18

    ('tree', '', 'c90aad02c6aa7df0113ccb215474c805633b552229da4af87ee9c5949ff1b73b'),  # protection views with PCID (plan Phase 5b), 2026-11-17


    ('timed', 'kernel/src/syscalls/mod.rs', 'fn-v1:dcfd9075428ef7e5f81f7f1b7bf46be288047256915f7d60347ddded728251d6'),  # driver restart: inbox watch + hang probe in syscall entry, 2026-11-19

)
