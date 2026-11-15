//! Parent-side offline adapter. Call poll at least once per UI loop and continue
//! polling after cancel until retired()==true; dropping is only best-effort kill.
use crate::job::{self, Identity, Job, Status};
use alloc::vec::Vec;
use sex_app::{files::direct as files, launch};
use sex_pdx::{restricted_helper::Subject, restricted_job as wire};
use sha2::{Digest, Sha256};
#[derive(Debug)]
pub enum Error {
    Memory,
    Job(job::Error),
    Path,
    Files,
    Launch,
    Result,
}
pub struct OfflineJob {
    job: Job,
    output: Vec<u8>,
    deadline: u64,
    last_kill: u64,
    retired: bool,
    published: bool,
    info: wire::Info,
    shell_epoch: u64,
    child: Option<Subject>,
    invalidated: bool,
    released: bool,
    last_status: u64,
    cleanup: Option<files::restricted::End>,
}
fn path_ok(p: &[u8]) -> bool {
    wire::path(p)
}
fn shell_epoch() -> Option<u64> {
    let peer = sex_pdx::crash_log::peer_state(sex_pdx::SLOT_SHELL).ok()?;
    (peer.live() && peer.epoch != 0).then_some(peer.epoch)
}
fn subject() -> Option<Subject> {
    let own = sex_pdx::app_lifecycle::catalog(sex_pdx::app_lifecycle::SELF_CATALOG_SLOT).ok()?;
    let row = sex_pdx::app_catalog::by_target(own.target_pd)?;
    (row.app == sex_pdx::app_catalog::App::Program && own.is_subject(row.target, row.surface, 3))
        .then_some(Subject {
            pd: own.target_pd,
            task: own.task_id,
        })
}
fn write_new(path: &[u8], bytes: &[u8], epoch: u64) -> Result<(), Error> {
    if files::restricted::stat(path, epoch).is_ok() {
        return Err(Error::Files);
    }
    files::restricted::create(path, epoch).map_err(|_| Error::Files)?;
    let mut at = 0;
    while at < bytes.len() {
        let n = files::restricted::write_at(path, at as u64, &bytes[at..], epoch)
            .map_err(|_| Error::Files)?;
        if n == 0 || n > bytes.len() - at {
            return Err(Error::Files);
        }
        at += n;
    }
    files::restricted::sync(epoch).map_err(|_| Error::Files)
}
impl OfflineJob {
    /// Paths must come from a user-granted render folder and use fresh nonce names.
    /// No whole-folder grant is passed to the helper: only these three exact paths.
    /// `nonce` must come from the parent's trusted CSPRNG, never timestamps/counters.
    pub fn start(
        plugin: &[u8],
        input: &[u8],
        output: &[u8],
        nonce: [u8; 16],
        cookie: u32,
        pcm: &[[f32; 2]],
        now: u64,
    ) -> Result<Self, Error> {
        Self::start_with(
            plugin,
            input,
            output,
            nonce,
            cookie,
            pcm,
            &[],
            job::Format::Clap,
            now,
        )
    }
    pub fn start_with(
        plugin: &[u8],
        input: &[u8],
        output: &[u8],
        nonce: [u8; 16],
        cookie: u32,
        pcm: &[[f32; 2]],
        parameters: &[job::Parameter],
        format: job::Format,
        now: u64,
    ) -> Result<Self, Error> {
        Self::start_restricted_with(
            plugin, input, output, nonce, cookie, pcm, parameters, format, now,
        )
    }
    /// Immutable Files snapshots and attested coordinator admission only.
    /// There is no ordinary-ticket or ambient-grant fallback on refusal.
    pub fn start_restricted_with(
        plugin: &[u8],
        input: &[u8],
        output: &[u8],
        nonce: [u8; 16],
        cookie: u32,
        pcm: &[[f32; 2]],
        parameters: &[job::Parameter],
        format: job::Format,
        now: u64,
    ) -> Result<Self, Error> {
        if ![plugin, input, output].iter().all(|p| path_ok(p))
            || plugin == input
            || input == output
            || plugin == output
        {
            return Err(Error::Path);
        }
        let job = Job::new(
            nonce,
            cookie,
            u32::try_from(pcm.len()).map_err(|_| Error::Result)?,
        )
        .map_err(Error::Job)?;
        let parent = subject().ok_or(Error::Launch)?;
        let shell = shell_epoch().ok_or(Error::Launch)?;
        let files_epoch = files::restricted::epoch().map_err(|_| Error::Files)?;
        if files::restricted::stat(output, files_epoch).is_ok() {
            return Err(Error::Files);
        }
        let deadline = now.checked_add(120_000).ok_or(Error::Launch)?;
        let request = job::encode_request(nonce, pcm, parameters).map_err(Error::Job)?;
        let input_hash: [u8; 32] = Sha256::digest(&request).into();
        write_new(input, &request, files_epoch)?;
        let mut output_path = Vec::new();
        output_path
            .try_reserve_exact(output.len())
            .map_err(|_| Error::Memory)?;
        output_path.extend_from_slice(output);
        let format_id = if format == job::Format::Clap { 0 } else { 1 };
        let info = files::restricted::prepare(&wire::Prepare {
            format: format_id,
            cookie,
            nonce,
            paths: [plugin, input, output],
        })
        .map_err(|_| Error::Files)?;
        let valid = info.parent == parent
            && info.frames == pcm.len() as u32
            && info.files_epoch == files_epoch
            && info.input_hash == input_hash
            && info.nonce == nonce
            && info.cookie == cookie
            && info.child.is_none()
            && info.output_hash == [0; 32]
            && info.format == format_id as u32;
        if !valid
            || shell_epoch() != Some(shell)
            || files::restricted::epoch().ok() != Some(files_epoch)
            || sex_pdx::get_ticks() >= deadline
        {
            let _ = files::restricted::release(info.ticket, nonce, files_epoch);
            return Err(Error::Result);
        }
        let words =
            wire::start_words(info.ticket, cookie, files_epoch, shell).ok_or(Error::Launch)?;
        if sex_pdx::pdx_call(
            sex_pdx::SLOT_SHELL,
            wire::OP_START,
            words[0],
            words[1],
            words[2],
        )
        .0 != 0
        {
            let _ = files::restricted::release(info.ticket, nonce, files_epoch);
            return Err(Error::Launch);
        }
        Ok(Self {
            job,
            output: output_path,
            deadline,
            last_kill: 0,
            retired: false,
            published: false,
            info,
            shell_epoch: shell,
            child: None,
            invalidated: false,
            released: false,
            last_status: 0,
            cleanup: None,
        })
    }
    /// Caller policy can shorten a deadline; it can never extend the 120s ceiling.
    pub fn shorten_deadline(&mut self, deadline: u64) {
        self.deadline = self.deadline.min(deadline);
    }
    pub fn cookie(&self) -> u32 {
        self.job.cookie()
    }
    /// Qualification observation only: the admitted Files ticket, its nonce and the
    /// held provider epochs. Holding these confers nothing once this parent retires.
    pub fn admission(&self) -> (u32, [u8; 16], u64, u64) {
        (self.info.ticket, self.info.nonce, self.info.files_epoch, self.shell_epoch)
    }
    pub fn status(&self) -> Status {
        self.job.status()
    }
    pub fn retired(&self) -> bool {
        self.retired
            && (self.released || (self.job.status() == Status::Complete && !self.published))
    }
    pub fn result_consumed(&self) -> bool {
        self.published
    }
    /// The retained authenticated Started identity, including after fast exit.
    /// This observation supplies no launch or lifecycle authority.
    pub fn child_identity(&self) -> Option<Identity> {
        self.child.map(|s| Identity {
            row: s.pd,
            task: s.task,
        })
    }
    pub fn cancel(&mut self) {
        self.job.cancel();
        if self.retired {
            self.published = true;
        }
        self.request_cancel();
    }
    fn poll_cleanup(&mut self, release: bool) {
        if self.released {
            return;
        }
        if files::restricted::epoch().is_ok_and(|epoch| epoch != self.info.files_epoch) {
            self.cleanup = None;
            self.released = true;
            return;
        }
        if self.cleanup.is_none() {
            match files::restricted::begin_end(
                self.info.ticket,
                self.info.nonce,
                self.info.files_epoch,
                release,
            ) {
                Ok(Some(pending)) => self.cleanup = Some(pending),
                Ok(None) => return,
                Err(_) => return,
            }
        }
        if let Some(result) = self.cleanup.as_mut().and_then(|pending| pending.poll()) {
            self.cleanup = None;
            self.released = result.is_ok();
        }
    }
    fn request_cancel(&self) {
        if let Some(shell) = shell_epoch() {
            if let Some(words) = wire::start_words(
                self.info.ticket,
                self.job.cookie(),
                self.info.files_epoch,
                shell,
            ) {
                let _ = sex_pdx::pdx_call(
                    sex_pdx::SLOT_SHELL,
                    wire::OP_CANCEL,
                    words[0],
                    words[1],
                    words[2],
                );
            }
        }
    }
    pub fn poll(&mut self, now: u64) {
        if files::restricted::epoch().ok() != Some(self.info.files_epoch)
            || shell_epoch() != Some(self.shell_epoch)
        {
            self.invalidated = true;
            self.job.cancel();
        }
        if !self.retired && now >= self.deadline {
            self.job.cancel();
        }
        while let Some(event) = launch::poll(self.job.cookie()) {
            match event {
                launch::Event::Started { cookie, row, task } => {
                    if row != self.info.parent.pd
                        && self.job.started(cookie, Identity { row, task }).is_ok()
                    {
                        self.child = Some(Subject { pd: row, task });
                    }
                }
                launch::Event::Done {
                    cookie,
                    row,
                    task,
                    status,
                } => {
                    if self
                        .job
                        .retired(cookie, Identity { row, task }, status)
                        .is_ok()
                    {
                        self.retired = true;
                    }
                }
                launch::Event::Refused { .. } => {
                    if self.child.is_none()
                        && matches!(self.job.status(), Status::Waiting | Status::CancelWaiting)
                    {
                        self.job.cancel();
                        self.retired = true;
                    }
                }
            }
        }
        if !self.retired
            && matches!(
                self.job.status(),
                Status::CancelWaiting | Status::Cancelling(_)
            )
            && now.saturating_sub(self.last_kill) >= 250
        {
            self.request_cancel();
            self.last_kill = now;
        }
        if matches!(
            self.job.status(),
            Status::CancelWaiting | Status::Cancelling(_)
        ) {
            self.poll_cleanup(false);
        }
        if !self.retired && self.invalidated {
            if let Some(shell) = shell_epoch() {
                if now.saturating_sub(self.last_status) >= 250 {
                    sex_app::restricted_status::query(
                        self.job.cookie(),
                        self.info.ticket,
                        self.child,
                        shell,
                    );
                    self.last_status = now;
                }
                if let Some(observed) =
                    sex_app::restricted_status::poll(self.job.cookie(), self.info.ticket, shell)
                {
                    if let Some(child) = self.child {
                        if observed.state == wire::STATUS_RETIRED
                            && observed.current.is_some_and(|s| s.pd == child.pd)
                            && shell_epoch() == Some(shell)
                        {
                            if self
                                .job
                                .retired(
                                    self.job.cookie(),
                                    Identity {
                                        row: child.pd,
                                        task: child.task,
                                    },
                                    None,
                                )
                                .is_ok()
                            {
                                self.retired = true;
                            }
                        }
                    }
                }
            }
        }
        if self.retired && (self.published || self.job.status() != Status::Complete) {
            self.poll_cleanup(true);
        }
    }
    /// Consumes at most one result, only after the matching successful retirement.
    pub fn read_result(&self) -> Result<Option<Vec<[f32; 2]>>, Error> {
        if self.published || !self.retired || self.job.status() != Status::Complete {
            return Ok(None);
        }
        if self.invalidated
            || files::restricted::epoch().ok() != Some(self.info.files_epoch)
            || shell_epoch() != Some(self.shell_epoch)
        {
            return Err(Error::Result);
        }
        let result =
            files::restricted::result(self.info.ticket, self.info.nonce, self.info.files_epoch)
                .map_err(|_| Error::Files)?;
        if result.ticket != self.info.ticket
            || result.nonce != self.info.nonce
            || result.parent != self.info.parent
            || result.cookie != self.info.cookie
            || result.frames != self.info.frames
            || result.format != self.info.format
            || result.files_epoch != self.info.files_epoch
            || result.coordinator_epoch != self.shell_epoch
            || result.child != self.child
            || result.child.is_none()
            || result.plugin_hash != self.info.plugin_hash
            || result.input_hash != self.info.input_hash
        {
            return Err(Error::Result);
        }
        let (dir, size) = files::restricted::stat(&self.output, self.info.files_epoch)
            .map_err(|_| Error::Files)?;
        if dir || size != job::HEADER as u64 + self.info.frames as u64 * 8 {
            return Err(Error::Result);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size as usize)
            .map_err(|_| Error::Memory)?;
        bytes.resize(size as usize, 0);
        let mut at = 0;
        while at < bytes.len() {
            let n = files::restricted::read_at(
                &self.output,
                at as u64,
                &mut bytes[at..],
                self.info.files_epoch,
            )
            .map_err(|_| Error::Files)?;
            if n == 0 || n > bytes.len() - at {
                return Err(Error::Result);
            }
            at += n;
        }
        let mut extra = [0];
        if files::restricted::read_at(&self.output, at as u64, &mut extra, self.info.files_epoch)
            .map_err(|_| Error::Files)?
            != 0
        {
            return Err(Error::Result);
        }
        if <[u8; 32]>::from(Sha256::digest(&bytes)) != result.output_hash
            || files::restricted::epoch().ok() != Some(self.info.files_epoch)
            || shell_epoch() != Some(self.shell_epoch)
        {
            return Err(Error::Result);
        }
        self.job.result(&bytes).map(Some).map_err(Error::Job)
    }
    pub fn accept_result(&mut self) {
        self.published = true;
    }
    pub fn take_result(&mut self) -> Result<Option<Vec<[f32; 2]>>, Error> {
        let result = self.read_result()?;
        if result.is_some() {
            self.accept_result();
        }
        Ok(result)
    }
}
impl Drop for OfflineJob {
    fn drop(&mut self) {
        if !self.retired {
            self.request_cancel();
        }
    }
}
