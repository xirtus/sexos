# Pipes, environment and working folder for child programs

Follow-up to `2026-11-14-std-process.md`. Asked by the Polyamour thread: `Stdio::piped()` and `output()` returned `Unsupported`, and children got no environment variables or working directory, so a program could not drive `python3` over stdin/stdout or capture a tool's output.

**Design.** No kernel change and no new syscall. The bytes go through Files, as a terminal's attached run already does (`sex_pdx::console_relay`): Files keeps rings, each side asks Files, and neither side maps the other's memory (DESIGN_GUARDRAILS §2). What is new is that **any** program may attach a child to one of its own relay slots, not only a `+launch` terminal. Such a child is a *pipe run*.

- **Protocol** (`crates/sex-pdx`, additions only):
  - `fs::PREPARE_PIPED = 2` is a PREPARE `offset` next to `PREPARE_LAUNCH`. `console_relay::pipe_prepare(slot, stdio)` packs the starter's slot and the stdio word (two bits per stream: `INHERIT` 0, `PIPED` 1, `NULL` 2). Files makes the same checks as for a launch (`program_prepare`), then attaches the child to that slot.
  - Relay ops 13 to 18:
    - `WRITE_ERR` (child stderr)
    - `TAKE_ERR` (starter reads stderr)
    - `ENV` (child reads its environment block)
    - `SET_ENV` and `SET_CWD` (starter, before the start)
    - `CLOSE_READ` (the starter stops reading a stream, so the child's writes there answer `HUNG_UP` and are dropped)
  - The starter reuses `PUT`, `TAKE`, `HANG_UP` and `RELEASE` on its slot.
  - `ATTACHED` answers `PIPE_RUN | stdio` for a pipe run. A terminal's run still gets 0.
- **Rings per pair** (Files): 4 KiB stdin, 32 KiB stdout (as before), plus 16 KiB stderr, a 4 KiB environment block and the working folder. There are still only as many pairs as program rows (9). A pipe run's unread output is never reclaimed for another run.
- **Working folder**: `SET_CWD` takes a path as the *starter* names it: in its own folder, or an `@/path` it holds. Files confines it with the starter's binding. At bind time Files adds it to the child's picked grants (`@/data/starter/work`), so the grant does not appear in the child's arguments. A folder the starter does not hold is refused (`NAME`, which becomes `NotFound`).
- **Ticket ownership** (security tightening): Files now records which program row prepared a ticket, and loads it only for that row. Shell's `OP_LAUNCH` now passes the launching program to Files instead of 0, as `OP_RUN` already did for terminals. Before this, a program could start another program's ticket by guessing its number. With pipes, that would have attached the child to someone else's slot.

**Library side:**
- `crates/sex-std/src/process.rs`: `spawn_with`/`Handed`, an 8-slot table with holder counts, and `pipe_read`/`pipe_write`/`pipe_close`. It also has the C ABI calls `sys_process_spawn_with`, `sys_process_pipe_{read,write,close}` and `sys_process_pause`.
  - A slot is given back (`RELEASE`) when the `Child` and all its pipes are gone and the run has ended.
  - A run that is still going keeps its slot until a later spawn finds that it ended.
- **Child side** (`crates/sex-std/src/sys.rs` and `console.rs`):
  - The starter adds the word `+piped`. `_start` removes it, asks Files (`ATTACHED` decides, not the word) and fetches the environment block.
  - It passes `envp` to std and sets the working folder.
  - Each stream goes to its pipe, is dropped (`NULL`), or stays the run's own console (`INHERIT`). A pipe run's `EXIT` marks the end of its output.
- `crates/sex-std/src/path.rs`: a working folder. Relative paths start there, `..` never leaves the program's folder or a granted folder, and `canonicalize` keeps `@` paths. `sys_sexos_getcwd`/`sys_sexos_chdir`/`sys_sexos_env` are there for std and C.
- **std** (`patches/rust-std/process_sexos.rs`):
  - `ChildPipe` is a real type (read, write, `read_to_end`, close on drop).
  - `spawn` maps `Stdio` to the stdio word. `output()` pipes stdout and stderr and makes stdin null.
  - `read_output` polls both streams, so neither can fill up while the parent waits on the other.
  - The environment (`env`, `env_remove`, `env_clear`, inherited) and `current_dir` are passed on.
  - Handing a `ChildStdout` or a `File` to another command is `Unsupported`.
- **std working folder** (`patches/rust-std/paths_sexos.rs`, new, added by `scripts/sex_std_source.py`): it replaces std's Hermit `getcwd` (always `/`) and the unsupported `chdir`. So `std::env::current_dir`/`set_current_dir` work.
- **C** (`crates/sex-libc`):
  - `environ` starts from the environment the starter handed over.
  - `posix_spawn`'s `envp` and `system()` (`environ`) pass the environment on.
  - `getcwd`/`chdir` use sex-std's working folder.

**Gate** (`scripts/process_gate.py`, CI "Programs run programs"):
- `run stdspawn` needs 23 checks. They replace "no pipes"/"output() unsupported" with:
  - `output()` (stdout, stderr, status)
  - 20 000 bytes through stdin and back
  - 300 000 + 70 000 bytes through both output streams at once
  - a dropped stdout
  - a broken pipe to an ended child
  - null stdin
  - a handed pipe is `Unsupported`
  - `env`, an inherited variable and `env_clear`
  - `current_dir` (own folder, and one that is not held)
  - `set_current_dir`
- `run chello spawn` needs 16 checks (added: `envp`, `system()` handing on `environ`, `getcwd`).
- Files must log `[files.pipe.attached]` and `[files.pipe.cwd] … cwd=@/data/stdspawn/work`.

**Left out:**
- A child's pipe handed straight to another child (`a | b` without the parent copying).
- "Inherit" still means the child's own console, not the parent's, so a hidden child of a program in Spindle's terminal prints to the serial record, not to that terminal.
- At most 8 pipe runs per program at once, and 9 program rows overall.
- No `popen`, and CPython's `subprocess` still has no fork/exec path. A program can drive `python3` from Rust `std::process`. From C, it can pass `envp` but cannot pipe yet.
- A Python script that turns `@/…` working folders into absolute paths sees `@/…` as relative.
