// SPDX-License-Identifier: GPL-3.0-only
//! Actual bounded encoder/Files/capture main-loop composition. No C callback
//! performs IPC. Every output turn remains owned until its exact Files reply.
extern crate alloc;
use crate::{
    capture::{Capture, Error as CaptureError},
    files::Files,
    session::{Phase, Session},
};
use alloc::vec::Vec;
use sex_media::recording::{Config, Drain, Encoder};
use sex_pdx::{desktop_recording as w, desktop_recording_capture as c, fs as f};
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Empty,
    Creating,
    Header,
    Prepared,
    Running,
    Trailer,
    Syncing,
    Done,
}
pub struct Runtime {
    pub session: Session,
    stage: Stage,
    encoder: Option<Encoder>,
    files: Option<Files>,
    capture: Option<Capture>,
    pixels: Vec<u32>,
    name: [u8; w::NAME_BYTES],
    name_len: usize,
    write_token: Option<(u64, usize)>,
    origin: Option<u64>,
    last_pts: Option<u64>,
    frames: u32,
    next_acquire: u64,
    failed: bool,
    report: Option<w::AppReport>,
    reported: Option<w::AppStatus>,
}
impl Runtime {
    pub const fn new() -> Self {
        Self {
            session: Session::new(),
            stage: Stage::Empty,
            encoder: None,
            files: None,
            capture: None,
            pixels: Vec::new(),
            name: [0; w::NAME_BYTES],
            name_len: 0,
            write_token: None,
            origin: None,
            last_pts: None,
            frames: 0,
            next_acquire: 0,
            failed: false,
            report: None,
            reported: None,
        }
    }
    pub fn stop(&mut self) {
        self.session.stop();
        if self
            .report
            .is_some_and(|r| r.status == w::AppStatus::Prepared)
        {
            self.report = None
        }
        if let Some(c) = self.capture.as_mut() {
            c.stop()
        }
    }
    pub fn can_exit(&self) -> bool {
        self.session.phase == Phase::Idle || self.stage == Stage::Done && self.report.is_none()
    }
    fn fail(&mut self) {
        self.failed = true;
        self.stop()
    }
    pub fn receive(&mut self, m: &sex_pdx::PdxMessage, own_task: u32) {
        if self.capture.as_mut().is_some_and(|c| c.receive(m)) {
            return;
        }
        if self.files.as_mut().is_some_and(|f| f.receive(m)) {
            return;
        }
        if m.type_id != w::OP_RECORDER {
            return;
        }
        let Ok(peer) = sex_pdx::crash_log::peer_state(sex_pdx::SLOT_SHELL) else {
            return;
        };
        if peer.life != sex_pdx::crash_log::PeerLife::Live {
            return;
        }
        if self.session.command(
            m.caller_pd,
            m.caller_task,
            [m.arg0, m.arg1, m.arg2],
            own_task,
            peer.epoch,
        ) {
            if self.session.phase == Phase::Stopping {
                self.stop()
            }
        }
    }
    fn queue_report(&mut self, status: w::AppStatus) {
        if self.reported == Some(status) || self.report.is_some_and(|r| r.status == status) {
            return;
        }
        self.report = self.session.report(status);
    }
    fn send_report(&mut self) {
        let Some(r) = self.report else { return };
        let Some(scope) = self.session.scope else {
            return;
        };
        let Ok(peer) = sex_pdx::crash_log::peer_state(sex_pdx::SLOT_SHELL) else {
            return;
        };
        if peer.epoch != scope.shell_epoch {
            self.report = None;
            return;
        }
        if peer.life != sex_pdx::crash_log::PeerLife::Live {
            return;
        }
        let a = r.args();
        match sex_pdx::pdx_call_checked(sex_pdx::SLOT_SHELL, w::OP_RECORDER, a[0], a[1], a[2]) {
            Ok(_) => {
                self.report = None;
                self.reported = Some(r.status)
            }
            Err(sex_pdx::ERR_SERVICE_NOT_READY) => {}
            Err(_) => self.report = None,
        }
    }
    fn prepare(&mut self) -> Result<(), ()> {
        let mut scope = self.session.scope.ok_or(())?;
        let peer = sex_pdx::crash_log::peer_state(sex_pdx::SLOT_DISPLAY).map_err(|_| ())?;
        let display_epoch = u32::try_from(peer.epoch).map_err(|_| ())?;
        if peer.life != sex_pdx::crash_log::PeerLife::Live
            || !self.session.observe_display(display_epoch)
        {
            return Err(());
        }
        scope.display_epoch = display_epoch;
        // Encoder availability is proved before CREATE, indicator or Acquire.
        let encoder = Encoder::new(Config {
            width: w::WIDTH,
            height: w::HEIGHT,
            fps: w::FPS,
        })
        .map_err(|_| ())?;
        let capture = Capture::open(c::Scope {
            session: scope.session,
            shell_epoch: scope.shell_epoch,
            display_epoch: scope.display_epoch as u64,
            record_task: scope.task,
            spindle_task: scope.spindle_task,
        })
        .map_err(|_| ())?;
        let mut files = Files::open(scope.files_epoch as u64).map_err(|_| ())?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(c::PIXELS).map_err(|_| ())?;
        pixels.resize(c::PIXELS, 0);
        let name = self.session.filename().ok_or(())?;
        self.name[..name.len()].copy_from_slice(name);
        self.name_len = name.len();
        files
            .start(f::CREATE, &self.name[..self.name_len], 0, &[])
            .map_err(|_| ())?;
        self.encoder = Some(encoder);
        self.capture = Some(capture);
        self.files = Some(files);
        self.pixels = pixels;
        self.stage = Stage::Creating;
        Ok(())
    }
    fn consume_file_reply(&mut self) {
        let Some(reply) = self.files.as_mut().and_then(Files::take_reply) else {
            return;
        };
        if reply.status != f::OK {
            self.fail();
            return;
        }
        match reply.kind {
            f::CREATE if self.stage == Stage::Creating => self.stage = Stage::Header,
            f::WRITE => {
                let Some((token, bytes)) = self.write_token.take() else {
                    self.fail();
                    return;
                };
                // Short writes are valid only for a nonzero prefix; encoder
                // re-advertises the rest under a fresh token after this ACK.
                if reply.value == 0
                    || reply.value as usize > bytes
                    || self
                        .encoder
                        .as_mut()
                        .is_none_or(|e| e.acknowledge(token, reply.value as usize).is_err())
                {
                    self.fail()
                }
            }
            f::SYNC
                if self.stage == Stage::Syncing
                    && self.capture.as_ref().is_some_and(Capture::released) =>
            {
                if self.session.saved() {
                    self.stage = Stage::Done;
                    if let Some(scope)=self.session.scope {
                        sex_pdx::serial_record!("[record.saved] session={} task={} frames={} last_pts_ms={} synced=1 released=1",scope.session.word(),scope.task,self.frames,self.last_pts.unwrap_or(0));
                    }
                    self.queue_report(w::AppStatus::Saved)
                } else {
                    self.fail()
                }
            }
            _ => self.fail(),
        }
    }
    fn drain(&mut self) -> Result<bool, ()> {
        let files = self.files.as_mut().ok_or(())?;
        if files.pending() {
            return Ok(false);
        }
        match self.encoder.as_ref().ok_or(())?.output().map_err(|_| ())? {
            Drain::Write(write) => {
                files
                    .start(
                        f::WRITE,
                        &self.name[..self.name_len],
                        write.offset,
                        write.bytes,
                    )
                    .map_err(|_| ())?;
                self.write_token = Some((write.token, write.bytes.len()));
                Ok(false)
            }
            Drain::NeedFrame => Ok(true),
            Drain::End => {
                if self.stage != Stage::Trailer {
                    return Err(());
                }
                files.start(f::SYNC, &[], 0, &[]).map_err(|_| ())?;
                self.stage = Stage::Syncing;
                Ok(false)
            }
        }
    }
    pub fn poll(&mut self, now: u64) {
        // Revocation transport precedes any C encoder operation or Files wait.
        if self.session.phase == Phase::Stopping {
            if let Some(c) = self.capture.as_mut() {
                c.stop()
            }
        }
        if let Some(c) = self.capture.as_mut() {
            if let Err(e) = c.poll() {
                if e != CaptureError::Unavailable {
                    self.fail()
                }
            }
        }
        if let Some(files) = self.files.as_mut() {
            files.poll();
            if files.fenced() {
                self.fail()
            }
        }
        self.consume_file_reply();
        if let Some(scope) = self.session.scope {
            if let Ok(peer) = sex_pdx::crash_log::peer_state(sex_pdx::SLOT_SHELL) {
                if peer.epoch != scope.shell_epoch
                    || peer.life != sex_pdx::crash_log::PeerLife::Live
                {
                    self.fail()
                }
            }
        }
        if self.stage == Stage::Empty && self.session.phase == Phase::Preparing {
            if self.prepare().is_err() {
                self.fail()
            }
        }
        if self.failed {
            // A failed destination stays named partial. Do not reuse or destroy
            // a Files buffer still owned by an unanswered request.
            let drained = self
                .files
                .as_ref()
                .is_none_or(|f| !f.pending() || f.fenced());
            let released = self
                .capture
                .as_ref()
                .is_none_or(|c| c.released() || c.fenced());
            if drained && released {
                self.session.fail();
                self.stage = Stage::Done;
                self.queue_report(w::AppStatus::Failed)
            }
            self.send_report();
            return;
        }
        if self.stage == Stage::Header
            || self.stage == Stage::Running
            || self.stage == Stage::Trailer
        {
            match self.drain() {
                Err(()) => self.fail(),
                Ok(true) if self.stage == Stage::Header => {
                    if self.session.phase == Phase::Preparing {
                        let epoch = self.capture.as_ref().unwrap().scope.display_epoch as u32;
                        if self.session.prepared(epoch) {
                            self.stage = Stage::Prepared;
                            self.queue_report(w::AppStatus::Prepared)
                        } else {
                            self.fail()
                        }
                    }
                }
                _ => {}
            }
        }
        if self.stage == Stage::Prepared && self.session.phase == Phase::Active {
            if self.capture.as_mut().unwrap().activate().is_ok() {
                self.stage = Stage::Running
            } else {
                self.fail()
            }
        }
        if self.session.phase == Phase::Stopping {
            let released = self.capture.as_ref().is_none_or(Capture::released);
            if released && self.files.as_ref().is_none_or(|f| !f.pending()) {
                if matches!(
                    self.stage,
                    Stage::Empty | Stage::Prepared | Stage::Header | Stage::Running
                ) {
                    let result = self
                        .encoder
                        .as_mut()
                        .ok_or(())
                        .and_then(|e| e.finish().map_err(|_| ()));
                    if result.is_ok() {
                        self.stage = Stage::Trailer;
                        self.queue_report(w::AppStatus::Stopped)
                    } else {
                        self.fail()
                    }
                }
            }
        } else if self.stage == Stage::Running
            && self.files.as_ref().is_some_and(|f| !f.pending())
            && self.write_token.is_none()
        {
            let ready = matches!(
                self.encoder.as_ref().unwrap().output(),
                Ok(Drain::NeedFrame)
            );
            if ready {
                let c = self.capture.as_mut().unwrap();
                if let Err(e) = c.copy_turn(&mut self.pixels) {
                    if e != CaptureError::Unavailable {
                        self.fail()
                    }
                }
                let frame = self.capture.as_mut().unwrap().take_frame();
                if let Some(frame) = frame {
                    let origin = *self.origin.get_or_insert(frame.publication_ms);
                    let pts = frame.publication_ms.checked_sub(origin).filter(|pts| {
                        *pts <= w::MAX_DURATION_MS && self.last_pts.is_none_or(|last| *pts > last)
                    });
                    if let Some(pts) = pts {
                        if self
                            .encoder
                            .as_mut()
                            .unwrap()
                            .frame(&self.pixels, pts)
                            .is_ok()
                        {
                            self.last_pts = Some(pts);
                            self.frames = self.frames.saturating_add(1);
                            // At most 301 receipts for the bounded 60s/5fps
                            // session, with no allocation or shared frame read.
                            if self.frames<=301 {if let Some(scope)=self.session.scope {
                                sex_pdx::serial_record!("[record.frame.accepted] session={} task={} index={} generation={} publication_ms={} pts_ms={}",scope.session.word(),scope.task,self.frames,frame.generation,frame.publication_ms,pts);
                            }}
                        } else {
                            self.fail()
                        }
                    } else {
                        self.fail()
                    }
                } else if !self.capture.as_ref().unwrap().pending() && now >= self.next_acquire {
                    self.next_acquire = now.saturating_add(c::INTERVAL_MS);
                    if let Err(e) = self.capture.as_mut().unwrap().acquire() {
                        if e != CaptureError::Unavailable {
                            self.fail()
                        }
                    }
                }
            }
        }
        self.send_report();
    }
}
