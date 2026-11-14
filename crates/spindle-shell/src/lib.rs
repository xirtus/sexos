// SPDX-License-Identifier: GPL-3.0-only
//! Spindle's shell: structured pipelines. Commands pass typed values (numbers, sizes,
//! durations, dates, text, lists, records, tables) through `|`; programs pass bytes.
//!
//! ```text
//! ls | where size > 1mb | sort-by name
//! open notes.json | get items | where done == false | select title due
//! ^rg TODO src | lines | length
//! let big = (ls | where size > 10mb); $big.name
//! seq 1 10 | math sum
//! build-world &          # a background job: jobs, fg 1, kill 1
//! ls | to json > listing.json
//! ```
//!
//! The crate is `no_std` + `alloc` and knows nothing about SexOS: every file, program
//! and job goes through the [`Host`] trait, so it runs the same in Spindle's terminal
//! and in host tests ([`MemHost`]).
//!
//! # The language
//!
//! - Statements are separated by `;` or newlines; stages by `|`. `# comment` to the end.
//! - Values: `42`, `0x2a`, `1.5`, `1mb` `10KiB` `2gb` (sizes; `kb`/`mb` count thousands,
//!   `kib`/`mib` 1024s), `5sec` `2min` `100ms` `1hr` `3day` (durations), `2026-11-14`
//!   (dates), `true`/`false`/`null`, `"text\n"` (escapes), `'raw'`, `` `bare word` ``,
//!   `[1 2 3]`, `{name: x, size: 3kb}`, `(pipeline or expression)`.
//! - `$name` and cell paths `$x.name.0`; `$in` is the stage's input, `$it` the row.
//! - `let x = pipeline` keeps a value for later lines.
//! - Row conditions (`where`, and the expression of `update`/`insert`): a bare word that
//!   names a column of the row is that column (`size > 1mb`); on the right of a
//!   comparison a bare word is text (`type == dir`). Operators: `== != < <= > >=`,
//!   `=~ !~` (contains; `^`/`$` anchor), `in not-in starts-with ends-with`,
//!   `and or xor not`, `+ - * / mod ** ++`.
//! - `^name` always runs a program; any name that is not a built-in is a program too,
//!   when [`Host::is_external`] says it exists.
//! - `> file` / `>> file` at the end of a pipeline write (append) its value, like
//!   `save -f` / `save -a`. Inside conditions `>` compares; use `| save file` there.
//! - `cmd &` at the end starts the last program as a background job.
//! - `help` lists the commands; `help where` explains one.
//!
//! # Integrating with a terminal
//!
//! ```ignore
//! let mut engine = spindle_shell::Engine::new();      // one per terminal session
//! // Enter pressed:
//! let mut step = engine.run(&line, &mut host, cols);
//! // In the event loop, until Step::Done:
//! match step {
//!     Step::Done(out) => screen.write(out.text),          // tables, values, errors
//!     Step::RunExternal { print, request } => {
//!         screen.write(print);                             // earlier statements' output
//!         start_program(&request);                         // asynchronously
//!         // ... when it exits:
//!         step = engine.resume(ExternalResult { output, exit_code }, &mut host);
//!     }
//!     Step::WaitJob { print, job, .. } => { /* like RunExternal, for `fg` */ }
//! }
//! ```
//!
//! - A program's request has `capture: false` when it is the last stage: the terminal
//!   should then show its output live (and give it the keyboard) and resume with an
//!   empty `output`. With `capture: true` its bytes feed the next stage (as text, or
//!   binary when not UTF-8), a `let` or a redirect.
//! - A structured value piped into a program reaches it as plain text
//!   ([`engine::to_bytes`]): tables as aligned columns, lists one item a line.
//! - Tab: [`Engine::complete`] gives replacements for the word under the cursor:
//!   commands, subcommands, flags, `$variables`, columns (by running the earlier
//!   read-only stages: `ls | where na<Tab>`), and paths.
//! - Colour as you type: [`Engine::highlight`] gives styled byte ranges; [`Style::ansi`]
//!   maps a style to an SGR sequence.
//! - `Engine::color` switches ANSI colour in output off for plain sinks.
//! - Ctrl+C while a program runs: stop it, then [`Engine::cancel`].
//! - Paths reach the host absolute and normalized; the terminal maps them to its
//!   filesystem (Term keeps its folder without the leading `/`).
//!
//! # Limits
//!
//! Programs inside `( )` are refused (run them as a stage of their own and `let` the
//! result); `=~` is a substring test, not a regex; `open` knows .json .toml .csv .tsv
//! and .txt (lines), anything else is text or binary; `save` converts .json .csv .tsv
//! only. Output of a program arrives whole, not streamed into later stages.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod ast;
mod builtins;
mod complete;
pub mod engine;
pub mod error;
mod eval;
pub mod formats;
pub mod highlight;
pub mod host;
pub mod lexer;
pub mod mem_host;
pub mod parser;
pub mod path;
pub mod render;
pub mod signature;
mod suggest;
pub mod units;
pub mod value;
pub mod width;

use alloc::vec::Vec;
use core::ops::Range;

pub use complete::{Completion, CompletionKind};
pub use engine::{Engine, ExternalRequest, ExternalResult, Output, Step};
pub use error::{ShellError, Span};
pub use highlight::Style;
pub use host::{DirEntry, EntryKind, Host, JobId, JobInfo, JobState, ProcessInfo};
pub use mem_host::MemHost;
pub use value::{PathMember, Record, Value};

impl Engine {
    /// Completions for the word that ends at `cursor` (a byte offset into `line`).
    pub fn complete(&self, line: &str, cursor: usize, host: &dyn Host) -> Vec<Completion> {
        complete::complete(line, cursor, host, self.vars())
    }

    /// Styled byte ranges of `line` for colouring it while it is typed.
    pub fn highlight(&self, line: &str) -> Vec<(Range<usize>, Style)> { highlight::highlight(line) }
}
