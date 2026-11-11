// SPDX-License-Identifier: GPL-3.0-only
//! Main-loop recorder orchestration. Domain/task numbers are correlation only;
//! all positive transitions require actual held lifecycle/peer observations.
//! No grant is assigned here; SEXOS_RECORDER stays off until BOOT/native proof.
use sex_pdx::{
    app_catalog as catalog, app_lifecycle as life,
    crash_log::{self, PeerLife},
    desktop_recording as w, desktop_recording_capture as c, program_store as store,
};
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Name,
    Read,
    Load,
    Release,
    Rebirth,
    Bind,
    Prepare,
    NameSend,
    AwaitPrepared,
    Arm,
    Active,
    Stopping,
    Done,
}
#[derive(Clone, Copy)]
struct Request {
    slot: u64,
    op: u64,
    args: [u64; 3],
    sent: bool,
}
#[derive(Clone, Copy)]
struct Person {
    task: u32,
    sequence: u32,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Observation {
    Live,
    Wait,
    Invalid,
}
pub struct Controller {
    phase: Phase,
    epoch: u64,
    session_counter: u32,
    role_counter: u32,
    person: Option<Person>,
    last_person: Option<Person>,
    session: Option<w::SessionSequence>,
    name: w::Name,
    files_epoch: u32,
    display_epoch: u32,
    task: u32,
    activation: usize,
    image_bytes: u32,
    request: Option<Request>,
    role: Option<w::RoleSequence>,
    image_held: bool,
    loaded: bool,
    name_offset: usize,
    stop: bool,
    display_stopped: bool,
    stop_sent: bool,
    app_stop_sent: bool,
    arm_sent: bool,
    terminal: Option<w::AppStatus>,
    close_needed: bool,
    deadline: u64,
    reply: Option<(Person, w::IntentStatus)>,
    candidate: Option<sex_pdx::PdxMessage>,
}
impl Controller {
    pub const fn new() -> Self {
        Self {
            phase: Phase::Idle,
            epoch: 0,
            session_counter: 0,
            role_counter: 0,
            person: None,
            last_person: None,
            session: None,
            name: w::Name::new(),
            files_epoch: 0,
            display_epoch: 0,
            task: 0,
            activation: 0,
            image_bytes: 0,
            request: None,
            role: None,
            image_held: false,
            loaded: false,
            name_offset: 0,
            stop: false,
            display_stopped: false,
            stop_sent: false,
            app_stop_sent: false,
            arm_sent: false,
            terminal: None,
            close_needed: false,
            deadline: 0,
            reply: None,
            candidate: None,
        }
    }
    pub fn pending(&self) -> bool {
        !matches!(self.phase, Phase::Idle | Phase::Done)
            || self.reply.is_some()
            || self.close_needed
    }
    fn role(&mut self) -> Option<w::RoleSequence> {
        let next = self.role_counter.checked_add(1)?;
        // High half belongs to the app; low last value is reserved for Stop.
        if next >= 0x7ffff {
            return None;
        }
        let r = w::RoleSequence::new(self.epoch, next)?;
        self.role_counter = next;
        Some(r)
    }
    fn scope(&self) -> Option<c::Scope> {
        Some(c::Scope {
            session: self.session?,
            shell_epoch: self.epoch,
            display_epoch: self.display_epoch as u64,
            record_task: self.task,
            spindle_task: self.person?.task,
        })
    }
    fn observe(target: u32, task: u32) -> Observation {
        let Some(row) = catalog::by_target(target) else {
            return Observation::Invalid;
        };
        let subject = match life::catalog(row.lifecycle_slot) {
            Ok(c) => c,
            Err(life::RETRY) => return Observation::Wait,
            Err(_) => return Observation::Invalid,
        };
        if !subject.is_subject(target, row.surface, w::SHELL) || !subject.is_sender(target, task) {
            return Observation::Invalid;
        }
        match life::info(row.lifecycle_slot) {
            Err(life::LIVE) => Observation::Live,
            Err(life::RETRY) => Observation::Wait,
            _ => Observation::Invalid,
        }
    }
    fn peer(slot: u64, epoch: u32) -> Observation {
        match crash_log::peer_state(slot) {
            Ok(p) if p.epoch != epoch as u64 => Observation::Invalid,
            Ok(p) if p.life == PeerLife::Live => Observation::Live,
            Ok(p) if matches!(p.life, PeerLife::Dead | PeerLife::GaveUp) => Observation::Invalid,
            _ => Observation::Wait,
        }
    }
    fn retain(&mut self, status: w::IntentStatus) {
        if let Some(p) = self.person {
            self.reply = Some((p, status))
        }
    }
    pub fn stop(&mut self) {
        if matches!(self.phase, Phase::Idle | Phase::Done) {
            return;
        }
        self.stop = true;
        if !self.arm_sent {
            // A pre-Prepare cancellation has no app session to report from.
            // Cooperative Close lets an idle app exit, or drains an in-flight
            // CREATE/WRITE before exit when Prepare already arrived.
            self.close_needed |= self.task != 0;
            self.display_stopped = true
        }
        if self.reply.is_some_and(|(_, status)| {
            matches!(status, w::IntentStatus::Active | w::IntentStatus::Starting)
        }) {
            self.reply = None
        }
        // Image custody still needs its exact cleanup; revocation runs first.
    }
    pub fn stop_now(&mut self) {
        self.stop();
        if self.stop {
            self.send_stop()
        }
    }
    pub fn stop_target(&self) -> bool {
        w::ENABLED && self.arm_sent && !self.display_stopped
    }
    /// Hide/close only after Stop was queued ahead of that Display operation.
    pub fn before_hide(&mut self, surface: u64) -> bool {
        if !w::ENABLED
            || catalog::by_target(w::TARGET).is_none_or(|row| row.frame() != surface)
            || !self.stop_target()
        {
            return true;
        }
        self.stop_now();
        self.stop_sent || self.display_stopped
    }
    fn finish(&mut self, status: w::IntentStatus) {
        self.phase = Phase::Done;
        self.request = None;
        self.candidate = None;
        self.close_needed = self.task != 0;
        self.retain(status)
    }
    fn queue(&mut self, slot: u64, op: u64, args: [u64; 3]) {
        self.request = Some(Request {
            slot,
            op,
            args,
            sent: false,
        });
        self.candidate = None
    }
    fn image_request(&mut self, kind: w::ImageKind) -> bool {
        // Release surrenders the exact image transaction, not a new nonce.
        let sequence = match kind {
            w::ImageKind::Read => self.role(),
            w::ImageKind::Release => self.role,
        };
        let Some(sequence) = sequence else {
            return false;
        };
        let Some(request) = w::ImageRequest::new(kind, sequence, self.epoch, self.files_epoch)
        else {
            return false;
        };
        self.role = Some(sequence);
        self.queue(store::SHELL_ROUTE_SLOT, w::OP_FILES_IMAGE, request.args());
        true
    }
    pub fn receive(&mut self, m: &sex_pdx::PdxMessage, now: u64, ordinary_busy: bool) -> bool {
        if !w::ENABLED {
            return false;
        }
        if m.type_id == w::OP_SPINDLE {
            if Self::observe(w::SPINDLE, m.caller_task) != Observation::Live
                || m.caller_pd != w::SPINDLE
            {
                return true;
            }
            let Some(intent) = w::Intent::decode(m.caller_pd, [m.arg0, m.arg1, m.arg2]) else {
                return true;
            };
            if let w::Intent::Stop { .. } = intent {
                self.stop();
                return true;
            }
            match intent {
                w::Intent::Begin {
                    sequence,
                    length,
                    shell_epoch,
                } => {
                    if ordinary_busy
                        || !matches!(self.phase, Phase::Idle | Phase::Done)
                        || self.reply.is_some()
                    {
                        return true;
                    }
                    let Ok(epoch) = crash_log::self_epoch() else {
                        return true;
                    };
                    if shell_epoch != epoch || w::RoleSequence::new(epoch, 1).is_none() {
                        return true;
                    }
                    if self
                        .last_person
                        .is_some_and(|old| old.task == m.caller_task && sequence <= old.sequence)
                    {
                        return true;
                    }
                    if self.epoch != 0 && self.epoch != epoch {
                        return true;
                    }
                    self.epoch = epoch;
                    self.person = Some(Person {
                        task: m.caller_task,
                        sequence,
                    });
                    self.last_person = self.person;
                    if !self.name.begin(m.caller_task, sequence, length as usize) {
                        return true;
                    }
                    self.phase = Phase::Name;
                    self.stop = false;
                    self.display_stopped = false;
                    self.stop_sent = false;
                    self.app_stop_sent = false;
                    self.arm_sent = false;
                    self.terminal = None;
                    self.close_needed = false;
                    self.image_held = false;
                    self.loaded = false;
                    self.task = 0;
                    self.session = None;
                    self.deadline = now.saturating_add(30_000);
                }
                w::Intent::Chunk {
                    sequence,
                    offset,
                    words,
                } if self.phase == Phase::Name => {
                    self.name
                        .chunk(m.caller_task, sequence, offset as usize, words);
                }
                w::Intent::Commit { sequence } if self.phase == Phase::Name => {
                    if self
                        .person
                        .is_none_or(|p| p.task != m.caller_task || p.sequence != sequence)
                        || self.name.complete(m.caller_task, sequence).is_none()
                    {
                        return true;
                    }
                    let Some(next) = self.session_counter.checked_add(1) else {
                        self.finish(w::IntentStatus::Unavailable);
                        return true;
                    };
                    let Some(session) = w::SessionSequence::new(self.epoch, next) else {
                        self.finish(w::IntentStatus::Unavailable);
                        return true;
                    };
                    let (Ok(files), Ok(display)) = (
                        crash_log::peer_state(store::SHELL_ROUTE_SLOT),
                        crash_log::peer_state(sex_pdx::SLOT_DISPLAY),
                    ) else {
                        return true;
                    };
                    let (Ok(fe), Ok(de)) =
                        (u32::try_from(files.epoch), u32::try_from(display.epoch))
                    else {
                        return true;
                    };
                    if files.life != PeerLife::Live || display.life != PeerLife::Live {
                        return true;
                    }
                    let Some(row) = catalog::by_target(w::TARGET) else {
                        return true;
                    };
                    let Some(slot) = row.activation_slot else {
                        return true;
                    };
                    let Ok(info) = life::activation(slot, false) else {
                        return true;
                    };
                    if !matches!(
                        info.state,
                        life::ACTIVATION_READY | life::ACTIVATION_REUSABLE
                    ) {
                        self.finish(w::IntentStatus::Busy);
                        return true;
                    }
                    self.session_counter = next;
                    self.session = Some(session);
                    self.files_epoch = fe;
                    self.display_epoch = de;
                    self.activation = slot;
                    if self.image_request(w::ImageKind::Read) {
                        self.phase = Phase::Read;
                        self.retain(w::IntentStatus::Starting)
                    } else {
                        self.finish(w::IntentStatus::Unavailable)
                    }
                }
                _ => {}
            }
            return true;
        }
        if m.type_id == 1 && m.caller_pd == w::FILES && m.caller_task == w::FILES {
            let belongs = w::ImageReply::decode(m.caller_pd, m.arg0).is_some()
                || w::BindReply::decode(m.caller_pd, m.arg0).is_some();
            if belongs && self.request.is_some_and(|r| r.sent) && self.candidate.is_none() {
                self.candidate = Some(*m)
            }
            return belongs;
        }
        if m.type_id == 1 && m.caller_pd == w::DISPLAY && m.caller_task == w::DISPLAY {
            if let Some(reply) = c::ControlReply::decode(m.caller_pd, m.arg0) {
                if self.stop
                    && reply.matches(
                        w::RoleSequence::new(self.epoch, 0x7ffff).unwrap(),
                        self.task,
                    )
                    && reply.status == c::ControlStatus::Stopped
                {
                    if Self::peer(sex_pdx::SLOT_DISPLAY, self.display_epoch) == Observation::Live {
                        self.display_stopped = true;
                        self.retain(w::IntentStatus::Stopped)
                    }
                } else if self.phase == Phase::Arm
                    && self.role.is_some_and(|r| reply.matches(r, self.task))
                    && self.candidate.is_none()
                {
                    self.candidate = Some(*m)
                }
                return true;
            }
        }
        if m.type_id == w::OP_RECORDER {
            let Some(report) = w::AppReport::decode(m.caller_pd, [m.arg0, m.arg1, m.arg2]) else {
                return false;
            };
            if report.task != m.caller_task
                || report.task != self.task
                || Some(report.session) != self.session
                || report.files_epoch != self.files_epoch
                || report.display_epoch != self.display_epoch
                || self.person.is_none_or(|p| p.task != report.spindle_task)
                || Self::observe(w::TARGET, self.task) != Observation::Live
            {
                return true;
            }
            match report.status {
                w::AppStatus::Prepared if self.phase == Phase::AwaitPrepared && !self.stop => {
                    let Some(sequence) = self.role() else {
                        self.stop();
                        return true;
                    };
                    let Some(scope) = self.scope() else {
                        self.stop();
                        return true;
                    };
                    let Some(args) = (c::ControlRequest {
                        kind: c::ControlKind::Arm,
                        sequence,
                        scope,
                    })
                    .args() else {
                        self.stop();
                        return true;
                    };
                    self.role = Some(sequence);
                    self.queue(sex_pdx::SLOT_DISPLAY, c::OP_CONTROL, args);
                    self.phase = Phase::Arm;
                }
                w::AppStatus::Saved => {
                    self.stop();
                    self.terminal = Some(w::AppStatus::Saved)
                }
                w::AppStatus::Failed => {
                    self.stop();
                    self.terminal = Some(w::AppStatus::Failed)
                }
                w::AppStatus::Stopped => self.stop(),
                _ => {}
            }
            return true;
        }
        if m.type_id == c::OP_EVENT && m.caller_pd == w::DISPLAY && m.caller_task == w::DISPLAY {
            if c::ClosedEvent::decode(m.caller_pd, [m.arg0, m.arg1, m.arg2])
                .is_some_and(|event| Some(event.scope) == self.scope())
                && Self::peer(sex_pdx::SLOT_DISPLAY, self.display_epoch) == Observation::Live
            {
                self.stop();
                self.display_stopped = true;
            }
            return true;
        }
        false
    }
    fn send_pending(&mut self) {
        let Some(r) = self.request.as_mut() else {
            return;
        };
        if r.sent {
            return;
        }
        match sex_pdx::pdx_call_checked(r.slot, r.op, r.args[0], r.args[1], r.args[2]) {
            Ok(_) => {
                r.sent = true;
                if self.phase == Phase::Arm {
                    self.arm_sent = true
                }
            }
            Err(sex_pdx::ERR_SERVICE_NOT_READY) => {}
            Err(_) => self.stop = true,
        }
    }
    fn send_stop(&mut self) {
        let Some(scope) = self.scope().filter(|s| s.valid()) else {
            return;
        };
        if !self.stop_sent
            && Self::peer(sex_pdx::SLOT_DISPLAY, self.display_epoch) == Observation::Live
        {
            if let Some(args) = (c::ControlRequest {
                kind: c::ControlKind::Stop,
                sequence: w::RoleSequence::new(self.epoch, 0x7ffff).unwrap(),
                scope,
            })
            .args()
            {
                if sex_pdx::pdx_call_checked(
                    sex_pdx::SLOT_DISPLAY,
                    c::OP_CONTROL,
                    args[0],
                    args[1],
                    args[2],
                )
                .is_ok()
                {
                    self.stop_sent = true
                }
            }
        }
        if !self.app_stop_sent && Self::observe(w::TARGET, self.task) == Observation::Live {
            if let Some(row) = catalog::by_target(w::TARGET) {
                let args = w::AppCommand::Stop {
                    session: scope.session,
                    shell_epoch: self.epoch,
                    display_epoch: self.display_epoch,
                    task: self.task,
                }
                .args();
                if sex_pdx::pdx_call_checked(
                    row.route_slot,
                    w::OP_RECORDER,
                    args[0],
                    args[1],
                    args[2],
                )
                .is_ok()
                {
                    self.app_stop_sent = true
                }
            }
        }
    }
    fn send_person(&mut self) {
        let Some((person, status)) = self.reply else {
            return;
        };
        match Self::observe(w::SPINDLE, person.task) {
            Observation::Wait => return,
            Observation::Invalid => {
                self.reply = None;
                return;
            }
            Observation::Live => {}
        }
        if self.stop && matches!(status, w::IntentStatus::Active | w::IntentStatus::Starting) {
            self.reply = None;
            return;
        }
        let (Some(word), Some(row)) = (
            w::intent_reply(person.sequence, status),
            catalog::by_target(w::SPINDLE),
        ) else {
            return;
        };
        if sex_pdx::pdx_call_checked(row.route_slot, 1, word, 0, 0).is_ok() {
            self.reply = None
        }
    }
    pub fn poll(&mut self, now: u64) {
        if !w::ENABLED {
            return;
        }
        if self.pending() && now >= self.deadline && self.deadline != 0 {
            self.stop()
        }
        if let Some(person) = self.person {
            if Self::observe(w::SPINDLE, person.task) == Observation::Invalid {
                self.stop()
            }
        }
        if self.task != 0
            && Self::observe(w::TARGET, self.task) == Observation::Invalid
            && !matches!(self.phase, Phase::Idle | Phase::Done)
        {
            self.stop();
            self.terminal = Some(w::AppStatus::Failed);
        }
        // Stop has its own retained transport, independent of the current Files
        // request and app/codec state. It is always attempted first.
        if self.stop {
            self.send_stop()
        }
        if self.close_needed {
            if let Some(row) = catalog::by_target(w::TARGET) {
                if Self::observe(w::TARGET, self.task) == Observation::Live {
                    if sex_pdx::pdx_call_checked(
                        row.close_slot,
                        life::OP_APP_CLOSE,
                        w::TARGET as u64,
                        self.task as u64,
                        life::CLOSE_CONFIRMED,
                    )
                    .is_ok()
                    {
                        self.close_needed = false
                    }
                } else if Self::observe(w::TARGET, self.task) == Observation::Invalid {
                    self.close_needed = false
                }
            }
        }
        if self.terminal.is_some() && !self.image_held && self.display_stopped {
            let status = if self.terminal == Some(w::AppStatus::Saved) {
                w::IntentStatus::Saved
            } else {
                w::IntentStatus::Failed
            };
            self.terminal = None;
            self.finish(status);
        }
        if matches!(self.phase, Phase::Idle | Phase::Done) {
            self.send_person();
            return;
        }
        if self.phase == Phase::Name && self.stop {
            self.finish(w::IntentStatus::Stopped);
            self.send_person();
            return;
        }
        // Name assembly has no peer epochs or shared-buffer custody yet.
        if self.phase == Phase::Name {
            self.send_person();
            return;
        }
        if Self::peer(store::SHELL_ROUTE_SLOT, self.files_epoch) != Observation::Live {
            // A restarted/unknown Files cannot bless a queued old read, binding,
            // or release reply. Retain image custody and refuse new work.
            self.stop();
            self.send_person();
            return;
        }
        if !self.stop && Self::peer(sex_pdx::SLOT_DISPLAY, self.display_epoch) != Observation::Live
        {
            self.stop();
            return;
        }
        if self.stop
            && !self.image_held
            && matches!(
                self.phase,
                Phase::Bind
                    | Phase::Prepare
                    | Phase::NameSend
                    | Phase::AwaitPrepared
                    | Phase::Arm
                    | Phase::Active
            )
        {
            self.request = None;
            self.candidate = None;
            self.phase = Phase::Stopping;
        }
        if self.phase == Phase::Load {
            if self.stop {
                if self.image_request(w::ImageKind::Release) {
                    self.phase = Phase::Release
                }
            } else {
                // Synchronous kernel private-copy/source check completes before
                // asking Files to surrender the shared image lease.
                let loaded = life::load_program(self.activation, self.image_bytes);
                self.loaded = loaded.is_ok();
                if !self.loaded {
                    self.stop()
                }
                if self.image_request(w::ImageKind::Release) {
                    self.phase = Phase::Release
                } else {
                    self.stop()
                }
            }
        }
        if self.phase == Phase::Rebirth {
            let info = match life::activation(self.activation, false) {
                Ok(i) => i,
                Err(_) => return,
            };
            let active = if info.state == life::ACTIVATION_READY {
                life::activation(self.activation, true)
            } else {
                life::rebirth(self.activation)
            };
            if let Ok(active) = active {
                if active.state == life::ACTIVATION_ACTIVE && active.successor == w::TARGET {
                    self.task = active.task;
                    if self.stop {
                        self.finish(w::IntentStatus::Stopped);
                        return;
                    }
                    let Some(sequence) = self.role() else {
                        self.stop();
                        return;
                    };
                    let Some(request) =
                        w::BindRequest::new(sequence, self.task, self.epoch, self.files_epoch)
                    else {
                        self.stop();
                        return;
                    };
                    self.role = Some(sequence);
                    self.queue(store::SHELL_ROUTE_SLOT, w::OP_FILES_BIND, request.args());
                    self.phase = Phase::Bind;
                }
            }
        }
        if self.phase == Phase::Prepare && !self.stop {
            let (Some(person), Some(session), Some(row)) =
                (self.person, self.session, catalog::by_target(w::TARGET))
            else {
                self.stop();
                return;
            };
            let Some(name) = self.name.complete(person.task, person.sequence) else {
                self.stop();
                return;
            };
            let args = w::AppCommand::Prepare {
                session,
                length: name.len() as u16,
                shell_epoch: self.epoch as u32,
                task: self.task,
                files_epoch: self.files_epoch,
                spindle_task: person.task,
            }
            .args();
            self.queue(row.route_slot, w::OP_RECORDER, args);
            self.phase = Phase::NameSend;
            self.name_offset = 0;
        }
        self.send_pending();
        if self.phase == Phase::NameSend && self.request.is_some_and(|r| r.sent) && !self.stop {
            let person = self.person.unwrap();
            let name = self.name.complete(person.task, person.sequence).unwrap();
            if self.name_offset == name.len() {
                self.request = None;
                self.phase = Phase::AwaitPrepared
            } else {
                let offset = self.name_offset;
                let n = (name.len() - offset).min(16);
                let mut chunk = [0u8; 16];
                chunk[..n].copy_from_slice(&name[offset..offset + n]);
                let args = w::AppCommand::Name {
                    session: self.session.unwrap(),
                    offset: offset as u8,
                    words: [
                        u64::from_le_bytes(chunk[..8].try_into().unwrap()),
                        u64::from_le_bytes(chunk[8..].try_into().unwrap()),
                    ],
                }
                .args();
                self.name_offset += n;
                self.queue(
                    catalog::by_target(w::TARGET).unwrap().route_slot,
                    w::OP_RECORDER,
                    args,
                );
            }
        }
        if let Some(m) = self.candidate.take() {
            match self.phase {
                Phase::Read | Phase::Release => {
                    let Some(reply) = w::ImageReply::decode(m.caller_pd, m.arg0) else {
                        return;
                    };
                    let expected = if self.phase == Phase::Read {
                        w::ImageKind::Read
                    } else {
                        w::ImageKind::Release
                    };
                    if Some(reply.sequence) != self.role || reply.kind != expected {
                        return;
                    }
                    self.request = None;
                    if reply.status != w::BindStatus::Ok {
                        if self.phase == Phase::Read {
                            self.finish(w::IntentStatus::Failed)
                        } else {
                            self.stop()
                        }
                    } else if self.phase == Phase::Read {
                        self.image_held = true;
                        self.image_bytes = reply.bytes;
                        self.phase = Phase::Load;
                    } else {
                        self.image_held = false;
                        if self.stop && !self.loaded {
                            self.finish(w::IntentStatus::Stopped)
                        } else {
                            self.phase = Phase::Rebirth
                        }
                    }
                }
                Phase::Bind => {
                    let Some(reply) = w::BindReply::decode(m.caller_pd, m.arg0) else {
                        return;
                    };
                    if Some(reply.sequence) != self.role || reply.task != self.task {
                        return;
                    }
                    self.request = None;
                    if reply.status == w::BindStatus::Ok {
                        self.phase = Phase::Prepare
                    } else {
                        self.stop()
                    }
                }
                Phase::Arm => {
                    let Some(reply) = c::ControlReply::decode(m.caller_pd, m.arg0) else {
                        return;
                    };
                    if !self.role.is_some_and(|r| reply.matches(r, self.task)) {
                        return;
                    }
                    self.request = None;
                    if reply.status == c::ControlStatus::Armed && !self.stop {
                        let args = w::AppCommand::Active {
                            session: self.session.unwrap(),
                            shell_epoch: self.epoch,
                            display_epoch: self.display_epoch,
                            task: self.task,
                        }
                        .args();
                        self.queue(
                            catalog::by_target(w::TARGET).unwrap().route_slot,
                            w::OP_RECORDER,
                            args,
                        );
                        self.phase = Phase::Active;
                        self.deadline = now.saturating_add(w::MAX_DURATION_MS);
                        self.retain(w::IntentStatus::Active);
                    } else {
                        self.stop()
                    }
                }
                _ => {}
            }
        }
        self.send_person();
    }
}
