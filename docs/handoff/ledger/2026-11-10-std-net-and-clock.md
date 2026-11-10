## Programs: std::net, C sockets and a real clock (2026-11-10)

Follow-up to workstream H (Rust std, POSIX guest library), on the program socket API of #69.
- **std::net.** `crates/sex-std/src/net.rs` answers Hermit's socket calls over the program's own
  network flows: `TcpStream` (connect, `connect_timeout`, read/write from two threads, peek,
  read/write timeouts, non-blocking, shutdown), `TcpListener` (bind on a fixed port, accept),
  `UdpSocket` (bind, send_to/recv_from, connect), `ToSocketAddrs` through the network
  service's resolver, `poll`. A socket is a descriptor beside files; waits happen a step at a
  time without holding the socket. Runs started without `run +net` get `EACCES`.
- **sex-app.** `net::direct`: window-free socket calls for any task. Net requests now take a
  program-wide turn (one in flight), fill and read the shared region inside it, and leave other
  tasks' answers and keys in the mailbox; flow numbers and sequence are atomics.
- **C.** `sex-libc/src/posix_net.rs`: musl's `socket`, `bind`, `listen`, `accept`, `connect`,
  `send`/`recv`, `sendto`/`recvfrom`, `shutdown`, `get/setsockopt`, `getpeername`,
  `getsockname`, `poll`, `fcntl` (O_NONBLOCK), `getaddrinfo`/`freeaddrinfo`/`gai_strerror`,
  `inet_pton`/`inet_ntop`/`inet_addr`, `htons` family, `gettimeofday`, translating musl's
  structures and numbers to sex-std's.
- **Civil time.** `SystemTime::now()`, C `time()`/`gettimeofday`/`clock_gettime(CLOCK_REALTIME)`
  now read the kernel's civil time: syscall 76, milliseconds since the Unix epoch, from the same
  checked q35 CMOS UTC read as the held clock source (`clock_source::sample`), read at most once a
  minute and carried forward by the millisecond tick. It is data like the tick, not a grant:
  every task may read it, it reaches no device on its own. Before, programs saw build time plus
  uptime.
- **Kernel growth** (named per DESIGN_GUARDRAILS §1): `kernel/src/clock_source.rs` +22 lines,
  `syscalls/mod.rs` +3, `ipc/sender.rs` +7, `program_workers.rs` +3. Why not a server: the CMOS is already read in ring 0 for the held source,
  and serving time from a server would give every program a route it otherwise does not hold.
- **Workers speak as their run.** `kernel/src/ipc/sender.rs` (+7) stamps a program worker's
  IPC with its run's main task, since a worker already shares the run's domain and authority
  (`program_workers::is_worker`, +3). Before, a `std::thread` that touched a socket was denied:
  the network service binds `+net` to the run's main task.
- **ABI change:** new syscall 76 (`sex_pdx::clock_source::civil::SYS`, `sex_pdx::civil_time_ms`);
  `clock_source::invoke` now shares `clock_source::sample` (same results). pins refreshed in
  `spec/abi_inputs.sha256`.
- **Gate.** `scripts/program_std_gate.py` (CI): host TCP+UDP echo services and a forwarded port;
  `run +net stdnet` (15 checks: clock within 10 minutes of the host's, connect, echo, two threads,
  timeouts, non-blocking, UDP, refused port, listener answering the gate), and
  `run +net chello net ...` (C TCP/UDP echo, poll); plain `chello` checks that a run without
  `+net` is refused.
