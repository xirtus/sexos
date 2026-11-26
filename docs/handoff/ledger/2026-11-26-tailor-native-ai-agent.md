# Tailor: Native Multi-Provider AI Coding Agent for SexOS

- **Date:** 2026-11-26
- **Branch:** `gemini/tailor-native-ai-agent`
- **Ring:** Pure ring-3 userspace (zero kernel changes)
- **Theme:** Fabric (`Tailor`: `crates/tailor-core` + `apps/tailor`)

## Summary

Built **Tailor**, SexOS's native multi-provider AI coding agent enabling Gemini, Claude, and Codex to inspect workspaces, propose edits, and run capability-scoped commands directly from inside SexOS.

### `crates/tailor-core` (`#![no_std]` + `alloc`, zero new external dependencies)
- **`src/json.rs`**: Minimal recursive JSON parser & encoder with depth cap (64), string escaping, and UTF-16 surrogate-pair unescaping.
- **`src/sse.rs`**: Incremental HTTP/1.1 + `Transfer-Encoding: chunked` / `Content-Length` + Server-Sent Events (`event:`, multi-line `data:`) decoder supporting arbitrary byte splits.
- **`src/provider.rs`**: Complete HTTP/1.1 POST request builders (with tool schemas) and incremental SSE stream parsers for:
  - **Gemini** (`generativelanguage.googleapis.com`, `/v1beta/models/{model}:streamGenerateContent?alt=sse`, `x-goog-api-key`)
  - **Claude** (`api.anthropic.com`, `/v1/messages`, `x-api-key`, `anthropic-version: 2023-06-01`, `stream: true`)
  - **Codex / OpenAI** (`api.openai.com`, `/v1/chat/completions`, `Authorization: Bearer ...`, `stream: true`)
- **`src/agent.rs`**: Multi-turn tool-calling state machine (`read_file`, `write_file`, `patch_file`, `list_dir`, `run_command`, `hotswap_server`) with explicit human-in-the-loop approval gating for all mutating tools and host-testable `Workspace` / `ToolRunner` traits.

### `apps/tailor` (`x86_64-sex-app.json` -> `iso_root/programs/tailor`)
- Native SexOS window app on `sex_app::Window`, non-blocking TLS 1.3 streaming via `sex_app::net::Duplex`, capability-scoped file tools via `sex_app::files`, and command launching via `sex_app::launch`.
- RAM-only zeroizing API key storage via `shibari::Keeper` (auto-loaded from `/data/tailor/{gemini,claude,codex}.key` or interactive `/key <provider> <secret>`).
- Interactive commands: `/provider`, `/model`, `/key`, `/approve` (`y`), `/deny` (`n`), `/clear`, `/help`, plus Up/Down scrolling and Ctrl+C stream cancellation.
- Registered in `Cargo.toml` (`[workspace].members`) and `sexos_build_spec.toml` (`[allowed].crates` + `build_tailor` stage).

## Verification
- `cargo test -p tailor-core` (10 unit tests: JSON roundtrip/surrogates/depth, chunked+SSE byte-by-byte & HTTP errors, Gemini/Claude/Codex request building & SSE parsing, multi-turn tool loop + approval gate).
- `cargo check --manifest-path apps/tailor/Cargo.toml --target x86_64-sex-app.json -Zbuild-std=core,alloc -Zbuild-std-features=compiler-builtins-mem`
- `python3 scripts/check_architectural_truth.py`
- `python3 scripts/check_tls_platform.py`
