// SPDX-License-Identifier: GPL-3.0-only
//! Pure application session admission. Observations are supplied by actual held
//! peer snapshots; this state never substitutes for capability authentication.
use sex_pdx::desktop_recording as w;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    pub session: w::SessionSequence,
    pub task: u32,
    pub spindle_task: u32,
    pub shell_epoch: u64,
    pub files_epoch: u32,
    pub display_epoch: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Name,
    Preparing,
    Prepared,
    Active,
    Stopping,
    Saved,
    Failed,
}
pub struct Session {
    pub scope: Option<Scope>,
    pub phase: Phase,
    name: w::Name,
}
impl Session {
    pub const fn new() -> Self {
        Self {
            scope: None,
            phase: Phase::Idle,
            name: w::Name::new(),
        }
    }
    /// `shell_epoch` must be the current Live held Shell peer and `own_task`
    /// the task admitted by this executable's self-catalog capability.
    pub fn command(
        &mut self,
        caller: u32,
        caller_task: u32,
        args: [u64; 3],
        own_task: u32,
        shell_epoch: u64,
    ) -> bool {
        if caller != w::SHELL || caller_task != w::SHELL || own_task == 0 {
            return false;
        }
        let Some(command) = w::AppCommand::decode(caller, args) else {
            return false;
        };
        match command {
            w::AppCommand::Prepare {
                session,
                length,
                shell_epoch: epoch,
                task,
                files_epoch,
                spindle_task,
            } => {
                if self.phase != Phase::Idle || task != own_task || epoch as u64 != shell_epoch {
                    return false;
                }
                if !self
                    .name
                    .begin(caller_task, session.word(), length as usize)
                {
                    return false;
                }
                self.scope = Some(Scope {
                    session,
                    task,
                    spindle_task,
                    shell_epoch,
                    files_epoch,
                    display_epoch: 0,
                });
                self.phase = Phase::Name;
                true
            }
            w::AppCommand::Name {
                session,
                offset,
                words,
            } => {
                let Some(scope) = self.scope else {
                    return false;
                };
                if self.phase != Phase::Name
                    || scope.session != session
                    || scope.task != own_task
                    || scope.shell_epoch != shell_epoch
                {
                    return false;
                }
                if !self
                    .name
                    .chunk(caller_task, session.word(), offset as usize, words)
                {
                    return false;
                }
                if self.name.complete(caller_task, session.word()).is_some() {
                    self.phase = Phase::Preparing
                }
                true
            }
            w::AppCommand::Active {
                session,
                shell_epoch: epoch,
                display_epoch,
                task,
            } => {
                let Some(scope) = self.scope else {
                    return false;
                };
                if self.phase != Phase::Prepared
                    || scope.session != session
                    || scope.task != task
                    || task != own_task
                    || scope.shell_epoch != shell_epoch
                    || epoch != shell_epoch
                    || scope.display_epoch != display_epoch
                {
                    return false;
                }
                self.phase = Phase::Active;
                true
            }
            w::AppCommand::Stop {
                session,
                shell_epoch: epoch,
                display_epoch,
                task,
            } => {
                let Some(scope) = self.scope else {
                    return false;
                };
                if scope.session != session
                    || scope.task != task
                    || task != own_task
                    || scope.shell_epoch != shell_epoch
                    || epoch != shell_epoch
                    || scope.display_epoch != 0 && scope.display_epoch != display_epoch
                {
                    return false;
                }
                self.stop();
                true
            }
        }
    }
    pub fn filename(&self) -> Option<&[u8]> {
        let scope = self.scope?;
        self.name.complete(w::SHELL, scope.session.word())
    }
    /// Only after encoder availability, private allocation, held bank validation,
    /// exclusive CREATE and complete header writes have all succeeded.
    pub fn observe_display(&mut self, display_epoch: u32) -> bool {
        if self.phase != Phase::Preparing || display_epoch == 0 {
            return false;
        }
        let Some(scope) = self.scope.as_mut() else {
            return false;
        };
        if scope.display_epoch != 0 && scope.display_epoch != display_epoch {
            return false;
        }
        scope.display_epoch = display_epoch;
        true
    }
    pub fn prepared(&mut self, display_epoch: u32) -> bool {
        if self.phase != Phase::Preparing || display_epoch == 0 {
            return false;
        }
        let Some(scope) = self.scope.as_mut() else {
            return false;
        };
        scope.display_epoch = display_epoch;
        self.phase = Phase::Prepared;
        true
    }
    pub fn stop(&mut self) {
        if !matches!(self.phase, Phase::Idle | Phase::Saved | Phase::Failed) {
            self.phase = Phase::Stopping
        }
    }
    pub fn fail(&mut self) {
        self.phase = Phase::Failed
    }
    /// Caller requires EOF, actual SYNC ACK and capture Release ACK. No protocol
    /// command can produce Saved or move a terminal session back to Active.
    pub fn saved(&mut self) -> bool {
        if self.phase != Phase::Stopping {
            return false;
        }
        self.phase = Phase::Saved;
        true
    }
    pub fn report(&self, status: w::AppStatus) -> Option<w::AppReport> {
        let scope = self.scope?;
        if scope.display_epoch == 0 {
            return None;
        }
        Some(w::AppReport {
            session: scope.session,
            status,
            task: scope.task,
            files_epoch: scope.files_epoch,
            display_epoch: scope.display_epoch,
            spindle_task: scope.spindle_task,
        })
    }
}
