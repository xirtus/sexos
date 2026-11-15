# std::process and C posix_spawn/waitpid/kill/system (slice I5, last item)

Builds on #223 (`sex_pdx::launch`, `sex_app::launch`): a program asks Files to PREPARE a run and Shell to start it. Shell answers STARTED/REFUSED and later DONE with the exit status (None when the run was killed or crashed). The standard libraries now sit on top of that. Everything here is library code inside the program (DESIGN_GUARDRAILS §2). There is no kernel change and no Shell change.

**How std gets a process backend.** std's Hermit port, which `x86_64-sex-std.json` uses, has no process layer at all. `std/src/sys/process/mod.rs` picks `unsupported.rs` for Hermit, and that file never calls into the program's library. So no answer in `crates/sex-std` can make `std::process::Command` work, which is why #176 had to put the fs extras in `sex_std::fs`. Process is different: it needs std itself to have a working `Command`, `Child` and `ExitStatus`, and only std can construct those types. Three pieces do that:

- `patches/rust-std/process_sexos.rs` is std's `unsupported.rs` for this toolchain with the run parts filled in. It calls `sys_process_{spawn,wait,kill,forget,self}`, which `crates/sex-std/src/process.rs` provides. It is MIT OR Apache-2.0, as std is.
- `scripts/sex_std_source.py` copies the toolchain's `library/` (rust-src) to `target/sex-std-src/library`. It adds that file as `sys/process/sexos.rs` and inserts one `cfg_select!` arm, `target_os = "hermit" => sexos`, before the `_ => unsupported` arm. Nothing else in std changes. The script uses hard links when it can and a copy (81 MB) when it cannot. It stamps the result, so a current copy is left alone. It stops with a clear message if a new toolchain's `mod.rs` no longer has the anchor.
- `scripts/sexos_build_trace.sh` (additions only) exports `__CARGO_TESTS_ONLY_SRC_ROOT=<that library>` for `x86_64-sex-std.json` stages and unsets it for every other stage. So every ordinary Rust or C program gets the new std; the kernel, servers and sex-app programs are untouched. `__CARGO_TESTS_ONLY_SRC_ROOT` is cargo's own override for the build-std source root. The toolchain is pinned, so it is stable here. If cargo ever drops it, std quietly falls back to `Unsupported`, and `program_std_gate`'s stdhello check and this gate both catch that.

**`std::process::Command`** (`crates/sex-std/src/process.rs` has the full rules):

- Program: a bare `NAME`, `/bin/NAME`, or `@/path` of a program file this run holds. Anything else, and a program that is not there, gives `NotFound`.
- Arguments: joined into one line with quoting that `sex_std::args::split` undoes (`args::join`, host-tested round trip). The line is limited to 256 bytes; past that the error is `ArgumentListTooLong`.
- Hidden or windowed. `run_hidden` (`RUN_HIDDEN`) means no window on any workspace, and before this change a hidden std program that printed would open a console nobody could see or close, then wait forever at exit. So the default is now a hidden helper that knows it is one. sex-std puts `+hidden` (`sex_std::process::HIDDEN_WORD`) first in its argument line, and the child's sex-std `_start` removes it before `main` and switches the console off: output goes to the serial record only, stdin is at its end, and the program never waits to be dismissed. With that, `Command::new("rg").arg("x").status()` runs rg to the end and returns its status, as a script expects. `sex_std::process::CommandExt::windowed(true)` gives the child its own window instead; its console then stays open until the person closes it, and `wait` returns after that. A program not built on sex-std that is started hidden from std sees `+hidden` as its first argument.
- Network: `CommandExt::network(true)` asks for `RUN_NETWORK`. Shell refuses the launch with `NOT_ALLOWED` unless the parent's run has the network, and spawn then fails with `PermissionDenied`. A child never gets the network by default, even from a parent that has it. `@/path` and `+packages` in the arguments are checked by Files, as before.
- `CommandExt` is carried through std as two environment names (`SEXOS_RUN_WINDOWED`, `SEXOS_RUN_NETWORK`). These are the only env entries read. Other env vars and `current_dir` have nothing to carry them between runs; they are accepted and ignored.
- `spawn` waits for Shell's answer, so a refusal is an error from `spawn`. A start Shell refuses as busy (`IMAGE_BUSY`, another program being read) is retried.
- Exit status: `Done{status: Some(n)}` gives `code() == Some(n)`. `None` (killed or crashed) gives `code() == None` and `success() == false`, which displays as "ended without a status (killed or crashed)".
- `Child::id()` is the child's task. `std::process::id()` is this run's task; before this change it panicked ("no pids on this platform"). `kill()` sends `OP_KILL` and leaves an already-ended run alone. `wait`/`try_wait` read the DONE event. One waiter takes every event for runs it started, so waits on several children from several threads all make progress.
- No pipes: `Stdio::piped()` and `InheritFile` give `Unsupported`, and so does `output()` unless stdout and stderr are both set to `null`/`inherit`. In that case `output()` returns the status and empty buffers, which is the honest answer since nothing could be captured. `inherit` means the child's own console, not the parent's.

**C (`crates/sex-libc/src/posix_process.rs`)** uses the same `sys_process_*` calls:

- `posix_spawn` and `posix_spawnp` take `/bin/NAME`, `NAME` or `@/path`; there is no `PATH` search because programs live in one store. `argv[1..]` become the arguments; `argv[0]` and `envp` are not passed on. Children start hidden. Spawn attributes are accepted and ignored (`posix_spawnattr_init/destroy/setflags/getflags`). File actions (`addopen`/`addclose`/`adddup2` record that one was added) make `posix_spawn` return `ENOSYS`, since runs share no descriptors.
- `waitpid` takes a pid (the child's task, or the cookie), or `-1`/`0` for any child, plus `WNOHANG`. A reaped child is forgotten, so waiting for it again gives `ECHILD`. Status uses the `<sys/wait.h>` encoding: `exit(n)` gives `WIFEXITED` with `WEXITSTATUS == n`, and killed or crashed gives `WIFSIGNALED` with `WTERMSIG == SIGKILL`. `wait` is also there.
- `kill(pid, SIGKILL|SIGTERM|SIGINT|SIGHUP|SIGQUIT)` ends a child; there are no handlers. Signal 0 checks whether the pid is still a child. A pid that is no child of this run gives `ESRCH`, and so does a process group. Other signals give `EINVAL`. `getpid` returns this run's task.
- `system(cmd)` runs the first word as the program and the rest as its arguments, split like plain shell words with `'...'`, `"..."` and `\`. There are no variables, globs, pipes or redirections. It returns a wait status; a program that cannot start gives `WEXITSTATUS == 127`, as from `sh`. `system(NULL)` returns 1. This replaces the old `ENOSYS` stub.
- `posix_util.rs` has `wait_status` and `command_words`, with host tests.
- `scripts/no_per_process_tables_gate.sh` forbids `fn waitpid`/`fn getpid` everywhere. It now sanctions exactly these two sex-libc functions, bound to the line that shows they call `sys_process_*`, under the same allowlist mechanism the gate already uses for sex-tls's `fork`.

**`sex_app::launch`** (additions): `run_with(name, args, network, hidden, cookie)` and `poll_where(|cookie| ...)` for a starter that follows several runs. sex-std's cookies start at `0x5E00_0000`, away from a program's own small ones.

**Programs.** `apps/stdspawn` is a test-only program, so it keeps a test name. It is both the checker and the child: `exit N`, `sleep S`, `args ...`. `apps/chello` gains `chello spawn` (the C checks). `apps/stdhello`'s old check, "Command says unsupported", would now start stdhello recursively; it now checks that a missing program gives `NotFound`, and `program_std_gate.py`'s docstring says so.

**Gate** (`scripts/process_gate.py`, CI step "Programs run programs" in "Programs, packages and ports"):

- `run stdspawn` must pass at least 13 checks:
  - `process::id`
  - status 7
  - `/bin/NAME` success
  - quoted arguments
  - `try_wait` gives `None` before the child ends, then `wait` succeeds
  - kill: no code, not success
  - missing program gives `NotFound`
  - a path outside the store gives `NotFound`
  - network refused with `PermissionDenied`
  - piped gives `Unsupported`
  - `output()` gives `Unsupported`
  - `output()` with null streams gives status 4
  - a windowed child gives status 3
- The gate also requires Shell's `launch.denied reason=network`, `launch.kill ok=1` and `launch.done status=Some(7)` and `status=None`. The hidden child's output must appear in the serial record.
- `run +net stdspawn net`: a run that has the network hands it on.
- `run chello spawn` must pass at least 13 checks:
  - `system` status 7
  - `system` with quoted words
  - `system` of a missing program gives 127
  - `system(NULL)`
  - `posix_spawnp`
  - `WNOHANG` gives 0
  - `kill` 0/`SIGKILL`
  - `waitpid` after the kill gives `WIFSIGNALED SIGKILL`
  - `waitpid` again gives `ECHILD`
  - `posix_spawn` + `waitpid` gives 5
  - `posix_spawn` of a missing program gives `ENOENT`
  - file actions give `ENOSYS`
  - `getpid`
- Each parent ends with status 0.

**What is left out:**
- No pipes between runs, so no captured `output()` and no `Stdio::piped`. The terminal relay (`sex_pdx::console_relay`, Files-held rings for `+launch` terminals) is the obvious base for them, but it is tied to the terminal grant today.
- No environment or working directory for children.
- No signals beyond "end it": `SIGTERM` is not catchable.
- No process groups. No `getppid`.
- A windowed std child that printed blocks its parent's `wait` until the person closes it.
- A child dropped without `wait` (a std `Child` dropped while running) leaves its DONE event in the mailbox. The mailbox is 32 slots and drops the oldest.
- A parent that holds `+launch` (a terminal) gets Files' terminal PREPARE path, so its children attach to the terminal and lose their grants.

**Gate output** (local run):
```
process_gate: {'ok': True, 'runs': [{'line': 'run stdspawn', 'target': 42, 'task': 42, 'passed': 13}, {'line': 'run +net stdspawn net', 'target': 42, 'task': 1073741830}, {'line': 'run chello spawn', 'target': 42, 'task': 1073741832, 'passed': 13}]}
program_std_gate: {'ok': True, ...} stdhello 22/22 twice, chello 32/32 twice, pdf, stdnet 15/15 twice, chello net 35/35
```
Tasks are large numbers (`1073741830` = 0x40000006); a C `pid_t` holds them while they stay below 2^31.
