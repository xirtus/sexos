//! Parent-side offline adapter. Call poll at least once per UI loop and continue
//! polling after cancel until retired()==true; dropping is only best-effort kill.
use crate::job::{self, Identity, Job, Status};
use alloc::vec::Vec;
use sex_app::{files::direct as files, launch};
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
}
fn path_ok(p: &[u8]) -> bool {
    p.starts_with(b"@/")
        && p.len() <= 192
        && !p.iter().any(|b| *b <= 32 || *b == 127)
        && !p.split(|b| *b == b'/').any(|s| s == b"." || s == b"..")
}
fn write_new(path: &[u8], bytes: &[u8]) -> Result<(), Error> {
    if files::stat(path).is_ok() {
        return Err(Error::Files);
    }
    files::create(path).map_err(|_| Error::Files)?;
    let mut at = 0;
    while at < bytes.len() {
        let n = files::write_at(path, at as u64, &bytes[at..]).map_err(|_| Error::Files)?;
        if n == 0 || n > bytes.len() - at {
            return Err(Error::Files);
        }
        at += n;
    }
    files::sync().map_err(|_| Error::Files)
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
        if files::stat(output).is_ok() {
            return Err(Error::Files);
        }
        let mut args = Vec::with_capacity(256);
        for b in nonce {
            args.push(b"0123456789abcdef"[(b >> 4) as usize]);
            args.push(b"0123456789abcdef"[(b & 15) as usize]);
        }
        for p in [plugin, input, output] {
            args.push(b' ');
            args.extend_from_slice(p);
        }
        args.push(b' ');
        args.extend_from_slice(format.token());
        if args.len() > 256 {
            return Err(Error::Path);
        }
        write_new(
            input,
            &job::encode_request(nonce, pcm, parameters).map_err(Error::Job)?,
        )?;
        launch::run_hidden(b"moanplug", &args, cookie).map_err(|_| Error::Launch)?;
        Ok(Self {
            job,
            output: output.to_vec(),
            deadline: now.saturating_add(120_000),
            last_kill: 0,
            retired: false,
            published: false,
        })
    }
    /// Caller policy can shorten a deadline; it can never extend the 120s ceiling.
    pub fn shorten_deadline(&mut self, deadline: u64) {
        self.deadline = self.deadline.min(deadline);
    }
    pub fn status(&self) -> Status {
        self.job.status()
    }
    pub fn retired(&self) -> bool {
        self.retired
    }
    pub fn cancel(&mut self) {
        self.job.cancel();
        launch::kill(self.job.cookie());
    }
    pub fn poll(&mut self, now: u64) {
        if !self.retired && now >= self.deadline {
            self.job.cancel();
        }
        while let Some(event) = launch::poll(self.job.cookie()) {
            match event {
                launch::Event::Started { cookie, row, task } => {
                    let _ = self.job.started(cookie, Identity { row, task });
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
                    self.job.cancel();
                    self.retired = true;
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
            launch::kill(self.job.cookie());
            self.last_kill = now;
        }
    }
    /// Consumes at most one result, only after the matching successful retirement.
    pub fn read_result(&self) -> Result<Option<Vec<[f32; 2]>>, Error> {
        if self.published || !self.retired || self.job.status() != Status::Complete {
            return Ok(None);
        }
        let s = files::stat(&self.output).map_err(|_| Error::Files)?;
        if s.dir || s.size > job::HEADER as u64 + job::MAX_FRAMES as u64 * 8 {
            return Err(Error::Result);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(s.size as usize)
            .map_err(|_| Error::Memory)?;
        bytes.resize(s.size as usize, 0);
        let mut at = 0;
        while at < bytes.len() {
            let n = files::read_at(&self.output, at as u64, &mut bytes[at..])
                .map_err(|_| Error::Files)?;
            if n == 0 || n > bytes.len() - at {
                return Err(Error::Result);
            }
            at += n;
        }
        let mut extra = [0];
        if files::read_at(&self.output, at as u64, &mut extra).map_err(|_| Error::Files)? != 0 {
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
            launch::kill(self.job.cookie());
        }
    }
}
