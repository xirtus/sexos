//! The complete file verb allowlist for an attested plugin run. No backend path
//! resolution occurs here: names select three already captured private buffers.
extern crate alloc;
use crate::restricted_job::{Blob, Error};
use crate::restricted_jobs::Entry;
use alloc::vec::Vec;
use sex_pdx::{
    fs as f,
    restricted_helper::{Binding, Subject},
};
use zeroize::Zeroizing;
fn bytes(data: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
    let mut v = Zeroizing::new(Vec::new());
    v.try_reserve_exact(data.len()).map_err(|_| Error::Memory)?;
    v.extend_from_slice(data);
    Ok(v)
}
pub fn blob(path: &[u8]) -> Result<Blob, Error> {
    match path {
        b"@/plugin" => Ok(Blob::Plugin),
        b"@/request" => Ok(Blob::Request),
        b"@/result" => Ok(Blob::Output),
        _ => Err(Error::Identity),
    }
}
pub enum Reply {
    Data(Zeroizing<Vec<u8>>),
    Count(usize),
    Publish,
}
pub fn run(
    entry: &mut Entry,
    actor: Subject,
    binding: Binding,
    live: bool,
    r: f::Request,
    path: &[u8],
    input: &[u8],
) -> Result<Reply, Error> {
    if r.kind == sex_pdx::restricted_job::KIND
        && r.offset == sex_pdx::restricted_job::BOOT_INFO
        && r.length == 0
        && path.is_empty()
    {
        entry.job.size(actor, binding, live, Blob::Plugin)?;
        return Ok(Reply::Data(bytes(&entry.info(false)?)?));
    }
    match r.kind {
        f::ARGS if path.is_empty() && r.offset == 0 && r.length == 0 => {
            entry.job.size(actor, binding, live, Blob::Plugin)?;
            let mut args = [0; 96];
            let nonce = entry.job.nonce();
            for (i, b) in nonce.into_iter().enumerate() {
                args[2 * i] = b"0123456789abcdef"[(b >> 4) as usize];
                args[2 * i + 1] = b"0123456789abcdef"[(b & 15) as usize];
            }
            let suffix = if entry.format == 0 {
                b" @/plugin @/request @/result clap".as_slice()
            } else {
                b" @/plugin @/request @/result vst3".as_slice()
            };
            args[32..32 + suffix.len()].copy_from_slice(suffix);
            Ok(Reply::Data(bytes(&args[..32 + suffix.len()])?))
        }
        f::STAT if r.offset == 0 && r.length == 0 => {
            let size = entry.job.size(actor, binding, live, blob(path)?)?;
            let mut stat = [0; 32];
            stat[8..16].copy_from_slice(&(size as u64).to_le_bytes());
            // Stable synthetic ID within this one job, never an ambient inode.
            stat[24..32].copy_from_slice(
                &(match blob(path)? {
                    Blob::Plugin => 1u64,
                    Blob::Request => 2,
                    Blob::Output => 3,
                })
                .to_le_bytes(),
            );
            Ok(Reply::Data(bytes(&stat)?))
        }
        f::READ => Ok(Reply::Data(bytes(entry.job.read(
            actor,
            binding,
            live,
            blob(path)?,
            r.offset,
            usize::try_from(r.length).map_err(|_| Error::Bounds)?,
        )?)?)),
        f::CREATE if r.offset == 0 && r.length == 0 => {
            entry.job.create(actor, binding, live, blob(path)?)?;
            Ok(Reply::Count(0))
        }
        f::TRUNCATE if r.length == 0 => {
            entry
                .job
                .truncate(actor, binding, live, blob(path)?, r.offset)?;
            Ok(Reply::Count(0))
        }
        f::WRITE if input.len() as u64 == r.length => Ok(Reply::Count(entry.job.write(
            actor,
            binding,
            live,
            blob(path)?,
            r.offset,
            input.len(),
            |i| input[i],
        )?)),
        f::SYNC if path.is_empty() && r.offset == 0 && r.length == 0 => {
            entry.job.seal(actor, binding, live)?;
            Ok(Reply::Publish)
        }
        _ => Err(Error::ReadOnly),
    }
}
