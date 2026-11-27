// SPDX-License-Identifier: GPL-3.0-only
//! Pluggable I/O traits for zero-rewrite portability across macOS/Linux/Windows and SexOS PDX.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::agent::{ToolRunner, Workspace};
use crate::provider::ProviderRequest;

/// A single match returned by [`WorkspaceFs::grep_search`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrepMatch {
    pub path: String,
    pub line: usize,
    pub text: String,
}

/// Incremental byte stream returned by [`HttpTransport::start_stream`].
pub trait HttpStream {
    /// Reads the next chunk into `buf`, returning `Ok(0)` on clean EOF.
    fn read_chunk(&mut self, buf: &mut [u8]) -> Result<usize, String>;
}

/// Pluggable HTTP/TLS transport for both buffered requests (OAuth refresh, etc.)
/// and streaming SSE completions.
pub trait HttpTransport {
    fn send_request(&mut self, req: &ProviderRequest) -> Result<Vec<u8>, String>;
    fn start_stream(&mut self, req: &ProviderRequest) -> Result<Box<dyn HttpStream>, String>;
}

/// Filesystem access scope controlling whether the agent is restricted to the
/// granted workspace or permitted to access the wider host filesystem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessScope {
    WorkspaceOnly,
    FullSystem,
}

impl Default for AccessScope {
    fn default() -> Self {
        Self::WorkspaceOnly
    }
}

impl AccessScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::WorkspaceOnly => "Workspace Only",
            Self::FullSystem => "Full System",
        }
    }
}

/// Extended workspace filesystem trait adding search and artifact helpers with
/// default fallbacks so existing [`Workspace`] implementations remain compatible.
pub trait WorkspaceFs: Workspace {
    fn find_by_name(&mut self, _dir: &str, _pattern: &str) -> Result<Vec<String>, String> {
        Err(String::from("find_by_name not supported"))
    }

    fn grep_search(&mut self, _path: &str, _query: &str) -> Result<Vec<GrepMatch>, String> {
        Err(String::from("grep_search not supported"))
    }

    fn read_artifact(&mut self, name: &str) -> Result<String, String> {
        self.read_file(name)
    }

    fn write_artifact(&mut self, name: &str, content: &str) -> Result<(), String> {
        self.write_file(name, content)
    }

    fn delete_file(&mut self, _path: &str) -> Result<(), String> {
        Err(String::from("delete_file not supported"))
    }

    fn move_file(&mut self, _from: &str, _to: &str) -> Result<(), String> {
        Err(String::from("move_file not supported"))
    }

    fn set_access_scope(&mut self, _scope: AccessScope) {}

    fn access_scope(&self) -> AccessScope {
        AccessScope::WorkspaceOnly
    }
}

/// Structured command execution result with exit code, stdout, and stderr.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Extended command execution trait with default fallback to [`ToolRunner::run_command`].
pub trait CommandRunner: ToolRunner {
    fn run_command_output(
        &mut self,
        program: &str,
        args: &str,
        network: bool,
    ) -> Result<CommandOutput, String> {
        let stdout = self.run_command(program, args, network)?;
        Ok(CommandOutput {
            exit_code: 0,
            stdout,
            stderr: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;

    struct DummyWs(BTreeMap<String, String>);

    impl Workspace for DummyWs {
        fn read_file(&mut self, path: &str) -> Result<String, String> {
            self.0
                .get(path)
                .cloned()
                .ok_or_else(|| String::from("not found"))
        }

        fn write_file(&mut self, path: &str, content: &str) -> Result<(), String> {
            self.0.insert(String::from(path), String::from(content));
            Ok(())
        }

        fn list_dir(&mut self, _path: &str) -> Result<Vec<String>, String> {
            Ok(self.0.keys().cloned().collect())
        }
    }

    impl WorkspaceFs for DummyWs {}

    struct DummyRunner;

    impl ToolRunner for DummyRunner {
        fn run_command(&mut self, program: &str, _args: &str, _network: bool) -> Result<String, String> {
            Ok(String::from(program))
        }

        fn hotswap_server(&mut self, _server: &str, _binary_path: &str) -> Result<String, String> {
            Ok(String::from("ok"))
        }
    }

    impl CommandRunner for DummyRunner {}

    #[test]
    fn default_workspace_fs_and_command_runner_fallbacks_work() {
        let mut ws = DummyWs(BTreeMap::new());
        assert!(ws.find_by_name(".", "*.rs").is_err());
        assert!(ws.grep_search(".", "fn").is_err());
        assert!(ws.delete_file("a.rs").is_err());
        assert!(ws.move_file("a.rs", "b.rs").is_err());
        assert_eq!(ws.access_scope(), AccessScope::WorkspaceOnly);
        ws.set_access_scope(AccessScope::FullSystem);
        assert_eq!(ws.access_scope(), AccessScope::WorkspaceOnly);
        assert_eq!(AccessScope::default(), AccessScope::WorkspaceOnly);
        assert_eq!(AccessScope::WorkspaceOnly.label(), "Workspace Only");
        assert_eq!(AccessScope::FullSystem.label(), "Full System");
        ws.write_artifact("plan.md", "# Plan").unwrap();
        assert_eq!(ws.read_artifact("plan.md").unwrap(), "# Plan");

        let mut runner = DummyRunner;
        let out = runner.run_command_output("cargo", "test", false).unwrap();
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout, "cargo");
        assert!(out.stderr.is_empty());
    }
}
