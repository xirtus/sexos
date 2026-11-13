# ripgrep and fd as ordinary programs (slice I7, rest)

**`apps/rg` and `apps/fd`.** ripgrep 15.2.0 and fd 10.5.2 run on SexOS as `run rg ...` and `run fd ...`. Both are the upstream programs, not rewrites: their binary sources are vendored from the crates.io packages (`ripgrep-15.2.0.crate`, `fd-find-10.5.2.crate`) and build on the published library crates they list (`grep`, `ignore`, `globset`, `regex`, `bstr`, `lexopt`, `termcolor`, `serde_json` for rg; `clap`, `ignore`, `globset`, `regex`, `lscolors`, `nu-ansi-term`, `crossbeam-channel`, `jiff`, `etcetera`, `normpath`, `faccess`, `argmax` for fd). Each app crate is only a `Cargo.toml` whose `[[bin]]` points at the vendored `main.rs`, plus the `build.rs` that gives ordinary programs an 8 MiB stack.

- `vendor/ripgrep-15.2.0/`: `crates/core` (the `rg` binary source) and its licence files (Unlicense OR MIT).
- `vendor/fd-10.5.2/`: `src` and its licence files (MIT OR Apache-2.0).

Every library crate builds for `x86_64-sex-std.json` (os = "hermit") as published, with one exception:

- `ignore` 0.4.34 is vendored (`vendor/ignore-0.4.34`, `[patch.crates-io]` in the workspace `Cargo.toml`). Its parallel walker answered every entry with "unsupported platform" off unix/windows (placeholder `DirEntryRaw::from_entry_os`/`from_path`). The two placeholders now build the entry from its path and file type, as on Unix without the inode (marked `SexOS:`). fd always walks in parallel, and rg does when it has more than one core, so without this fd found nothing.

`crates/sex-std`: `fs::canonical`/`canonicalize` keep a granted path as `@/absolute/path` (it was turned into `/@/...`, which Files does not grant), and refuse `..` in one. fd checks a search path with no file name (`@/`) by canonicalizing it.

The other SexOS changes are in the vendored binary sources, each marked with a `// SexOS:` comment:

ripgrep:
- `main.rs`: `use sex_std as _;` (links the std system interface).
- `haystack.rs`: a walk error with OS code 0 is skipped silently. std's Hermit `read_dir` reports an empty folder that way ("no entries" read as a failure); without this every empty folder printed an error and turned the exit status into 2.

fd:
- `main.rs`: `use sex_std as _;`; `one_file_system` is false where the `--one-file-system` flag doesn't exist (neither unix nor windows); colours only when `TERM` is set and not `dumb` (the program's window shows text, not ANSI escapes, and fd's non-Windows default assumed a colour terminal).
- `walk.rs`: no Ctrl-C handler (no signals; `ctrlc` is now a unix/windows-only dependency in `apps/fd/Cargo.toml`), and the same empty-folder skip as rg.
- `filesystem.rs`: `is_block_device`/`is_char_device`/`is_socket`/`is_pipe` answer false off unix (was windows-only), and `normalize` uses `sex_std::fs::canonicalize` on hermit, because std's Hermit backend refuses `canonicalize` (which `normpath` calls). Without it `fd PATTERN` with no path failed on `./`.
- `cli.rs`: `--absolute-path` goes through that `normalize`.

**Registration** (append-only): workspace `Cargo.toml` members; `sexos_build_spec.toml` whitelist entries and the `build_rg`/`build_fd` stages (`iso_root/programs/rg`, `iso_root/programs/fd`); `Cargo.lock` gains the new packages (no existing version moved). Sizes: rg 5.6 MB, fd 4.0 MB (program store limit 16 MB).

**Gate** (`scripts/rg_fd_gate.py`, CI step "ripgrep and fd" after the Lua step). It installs both programs on a blank disk and types everything into Spindle, reusing `lua_gate.py`'s key-by-key typer:
1. `run rg --version` prints `ripgrep 15.2.0`; Files makes `/data/rg`.
2. Spindle's own `mkdir`/`write`/`append` build `/data/rg/t`: `README.md` (2 lines), `src/main.rs`, `src/lib.rs`, `docs/notes.md`, `docs/todo.txt`, and an empty folder `empty`.
3. rg (`--sort path` for a fixed order): `-n --no-heading hello t` (file:line:text), `-i`, `-c -i`, `-g *.md`, `-l todo` with no path (the program's own folder), and `zebra t` exiting with status 1. Every run must print nothing on standard error (so the empty folder is not reported).
4. fd, from its own folder `/data/fd`, given the tree as a granted folder `@/data/rg/t`: `main`, `-e md .`, `-t d .` (includes the empty folder), compared as sets since fd walks on several threads; and `run fd` with no arguments in its own empty folder prints nothing and exits 0.

**What doesn't work, and why:**
- `rg --pre`, `rg -z/--search-zip` and `fd -x/-X/--exec`, `fd -l` start other programs; `std::process::Command` is Unsupported on SexOS (see `sex_std::process`), so they report an error.
- No memory maps: `memmap2` builds its stub off unix/windows, so when rg's heuristic tries a map (`--mmap`, or a few explicit file paths) it fails and rg falls back to ordinary reads, silently.
- No PCRE2 (`rg -P`): it is a C library and the `pcre2` feature stays off.
- No colours by default: the window console doesn't interpret ANSI. `--color always` still emits escapes.
- `fd --owner`, `--one-file-system` and the special file types (`-t b/c/s/p`) don't exist off unix; everything is a file or folder on SexOS anyway.
- `fd --base-directory` fails: std's Hermit `set_current_dir` is unsupported.
- Ctrl-C does not stop a search (no signals); closing the window does.
- Global ignore files (`~/.config/fd/ignore`, `$XDG_CONFIG_HOME/git/ignore`) are never found: there is no home folder or environment.
- Paths a program is granted show as `@/absolute/path` in output, because that is how the program names them.

**Gate output** (local run, one core; `scripts/check_tls_platform.py` now lists the reviewed `ignore` patch):

```
$ run rg --sort path -n --no-heading hello t  (status 0)
  t/README.md:2:second line says hello
  t/src/lib.rs:1:pub fn greet() { hello(); }
$ run rg --sort path -n --no-heading -i hello t  (status 0)
  t/README.md:1:Hello world
  t/README.md:2:second line says hello
  t/docs/notes.md:1:notes about HELLO
  t/src/lib.rs:1:pub fn greet() { hello(); }
$ run rg --sort path -c -i hello t  (status 0)
  t/README.md:2
  t/docs/notes.md:1
  t/src/lib.rs:1
$ run rg --sort path -n --no-heading -i -g *.md hello t  (status 0)
  t/README.md:1:Hello world
  t/README.md:2:second line says hello
  t/docs/notes.md:1:notes about HELLO
$ run rg -l todo  (status 0)
  t/docs/todo.txt
$ run rg zebra t  (status 1)
$ run fd main @/data/rg/t  (status 0)
  @/data/rg/t/src/main.rs
$ run fd -e md . @/data/rg/t  (status 0)
  @/data/rg/t/README.md
  @/data/rg/t/docs/notes.md
$ run fd -t d . @/data/rg/t  (status 0)
  @/data/rg/t/docs/
  @/data/rg/t/empty/
  @/data/rg/t/src/
$ run fd   (status 0)
{'ok': True, 'runs': 11}
```
