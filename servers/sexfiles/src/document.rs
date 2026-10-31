//! Service-owned immutable document snapshots, isolated by exact BOOT incarnation.
use sex_pdx::document as w;
use sex_pdx::app_catalog::{self, DOCUMENT_COUNT, DOCUMENT_TARGETS};
pub trait Backend {
    fn identity(&mut self, id: u8) -> Result<u32, u32>;
    fn load(&mut self, id: u8, out: &mut [u8; w::MAX]) -> Result<usize, u32>;
    fn cancel_load(&mut self, _caller:u32) {}
    fn publish_load(&mut self, _caller:u32, _request:w::Request, _generation:u32,
        _name:[u8;16], _bytes:&[u8]) -> Result<(),u32> { Err(w::DENIED) }
    fn clear_load(&mut self, _caller:u32) -> Result<(),u32> { Ok(()) }
    fn load_step(&mut self, _caller:u32, id:u8, out:&mut[u8;w::MAX])->Result<Option<usize>,u32>{
        self.load(id,out).map(Some)
    }
    fn name(&mut self, id: u8) -> Result<[u8; 16], u32>;
    fn create(&mut self, _name: [u8; 16]) -> Result<u8, u32> {
        Err(w::DENIED)
    }
    /// Explicit fresh-create contract; never an open-or-create operation.
    /// Return the service identity of the newly allocated object atomically.
    fn create_new(&mut self, _name:[u8;16])->Result<(u8,u32),u32>{Err(w::DENIED)}
    fn cancel_save(&mut self, _caller: u32) {}
    fn save_step(&mut self, caller: u32, id: u8, bytes: &[u8]) -> Result<bool, u32> {
        self.save(caller, id, bytes).map(|_| true)
    }
    fn save(&mut self, _caller: u32, _id: u8, _bytes: &[u8]) -> Result<(), u32> {
        Err(w::DENIED)
    }
}
/// Four data requests may be admitted on the existing BOOT route.
/// Each owns its receipt slot before any backend or staging effect.
const RECEIPTS: usize = w::WINDOW;
// Participant indices isolate immutable BOOT incarnations; IDs are not grants.
#[derive(Clone, Copy)]
struct Receipt {
    request: w::Request,
    value: Option<u64>,
}
struct Session {
    bytes: [u8; w::MAX],
    name: [u8; 16],
    len: usize,
    object: u8,
    generation: u32,
    bound: bool,
    saving: bool,
    commit: Option<w::Request>,
    loading: Option<w::Request>,
    written: usize,
    last: Option<w::Request>,
    receipt: u64,
    pending: [Option<Receipt>; RECEIPTS],
    pending_len: usize,
    dead: bool,
    created:Option<(u8,u32)>,
}
impl Session {
    const fn new() -> Self {
        Self {
            bytes: [0; w::MAX],
            name: [0; 16],
            len: 0,
            object: 0,
            generation: 0,
            bound: false,
            saving: false,
            commit: None,
            loading: None,
            written: 0,
            last: None,
            receipt: 0,
            pending: [None; RECEIPTS],
            pending_len: 0,
            dead: false,
            created:None,
        }
    }
    fn clear(&mut self) {
        self.bytes.fill(0);
        self.name.fill(0);
        self.len = 0;
        self.bound = false;
        self.saving = false;
        self.commit = None;
        self.loading = None;
        self.written = 0;
        self.created=None;
    }
}
pub struct Service {
    sessions: [Session; DOCUMENT_COUNT],
}
impl Service {
    pub const fn new() -> Self {
        Self {
            sessions: [const { Session::new() }; DOCUMENT_COUNT],
        }
    }
    fn index(caller: u32) -> Option<usize> {
        DOCUMENT_TARGETS.iter().position(|subject|*subject==caller)
    }
    #[cfg(test)]
    pub fn browser_import_scrubbed(&self)->bool{
        DOCUMENT_TARGETS.iter().enumerate().all(|(index,target)|{
            if !app_catalog::is_browser(*target){return true}
            let s=&self.sessions[index];s.created.is_none()&&!s.bound&&!s.saving
                &&s.commit.is_none()&&s.loading.is_none()&&s.bytes.iter().all(|b|*b==0)
                &&s.name.iter().all(|b|*b==0)&&s.len==0&&s.written==0
        })
    }
    pub fn pending(&self) -> bool {
        self.sessions
            .iter()
            .any(|s| s.pending_len != 0 || s.commit.is_some() || s.loading.is_some())
    }
    /// Called only after queued messages (especially terminal notices) drain.
    /// Admission and completion are separate; a duplicate cannot replay COMMIT.
    pub fn progress(&mut self, b: &mut impl Backend) {
        for (i, s) in self.sessions.iter_mut().enumerate() {
            if let Some(r)=s.loading {
                let caller=DOCUMENT_TARGETS[i];
                let result=if b.identity(s.object)!=Ok(s.generation){Err(w::STALE)}
                    else{b.load_step(caller,s.object,&mut s.bytes)};
                match result{
                    Ok(None)=>{},
                    done=>{
                        let result=done.and_then(|len|{
                            let len=len.ok_or(w::IO)?;
                            if len>w::MAX||b.identity(s.object)!=Ok(s.generation){return Err(w::STALE)}
                            s.name=b.name(s.object)?;
                            if r.b == w::SNAPSHOT_MODE {
                                b.publish_load(caller,r,s.generation,s.name,&s.bytes[..len])?;
                            }
                            s.len=len;s.bound=true;s.loading=None;
                            sex_pdx::serial_record!("[files.document.bound] object={} generation={} bytes={}",s.object,s.generation,len);
                            Ok(len as u32)
                        });
                        b.cancel_load(caller);
                        if result.is_err(){let _=b.clear_load(caller);s.clear();}
                        let v=w::reply(r.seq,result);s.receipt=v;
                        for entry in s.pending[..s.pending_len].iter_mut().flatten(){
                            if entry.request==r{entry.value=Some(v);break}
                        }
                    }
                }
                break;
            }
            if let Some(r) = s.commit {
                let caller = DOCUMENT_TARGETS[i];
                let result = if b.identity(s.object) != Ok(s.generation) {
                    Err(w::STALE)
                } else {
                    b.save_step(caller, s.object, &s.bytes[..s.len])
                };
                match result {
                    Ok(false) => {}
                    done => {
                        b.cancel_save(caller);
                        s.commit = None;
                        let result = done.map(|_| {
                            s.saving = false;
                            s.len as u32
                        });
                        let v = w::reply(r.seq, result);
                        s.receipt = v;
                        // COMMIT reserved this entry at admission. Never allocate
                        // a completion slot after the persistent effect.
                        for entry in s.pending[..s.pending_len].iter_mut().flatten() {
                            if entry.request == r {
                                entry.value = Some(v);
                                break;
                            }
                        }
                    }
                }
                // One transaction step per turn, irrespective of client count.
                break;
            }
        }
    }
    pub fn poll(&mut self, mut send: impl FnMut(u32, u64) -> u64) {
        for (i, s) in self.sessions.iter_mut().enumerate() {
            while s.pending_len != 0 {
                let Some(v) = s.pending[0].and_then(|entry| entry.value) else {
                    break;
                };
                // The legacy reply syscall returns 1 on Full. Every nonzero
                // result retains the exact head and all subsequent receipts.
                if send(DOCUMENT_TARGETS[i], v) != 0 {
                    break;
                }
                s.pending.copy_within(1..s.pending_len, 0);
                s.pending_len -= 1;
                s.pending[s.pending_len] = None;
            }
        }
    }
    pub fn retire(&mut self, caller: u32, b: &mut impl Backend) -> bool {
        b.cancel_save(caller);
        b.cancel_load(caller);
        if let Some(i) = Self::index(caller) {
            if b.clear_load(caller).is_err() { return false; }
            let s = &mut self.sessions[i];
            s.clear();
            s.pending = [None; RECEIPTS];
            s.pending_len = 0;
            s.last = None;
            s.dead = true;
            sex_pdx::serial_record!(
                "[files.document.retire] target={} staged=0 pending=0 terminal=1",
                caller
            );
        }
        true
    }
    fn execute(
        s: &mut Session,
        caller: u32,
        r: w::Request,
        b: &mut impl Backend,
    ) -> Result<u32, u32> {
        if r.kind == w::ABORT {
            if r.object != 0 || r.offset != 0 || r.a != 0 || r.b != 0 {
                return Err(w::INVALID);
            }
            b.cancel_save(caller);
            b.cancel_load(caller);
            b.clear_load(caller)?;
            s.clear();
            return Ok(0);
        }
        if r.kind == w::CREATE {
            if s.bound || (app_catalog::is_browser(caller)&&s.created.is_some()) || r.object != 0 || r.offset != 0 {
                return Err(w::BUSY);
            }
            let mut name = [0; 16];
            name[..8].copy_from_slice(&r.a.to_le_bytes());
            name[8..].copy_from_slice(&r.b.to_le_bytes());
            let n = name.iter().position(|&b| b == 0).unwrap_or(16);
            if n == 0
                || !name[..n]
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(*b, b'-' | b'_' | b'.'))
                || name[n..].iter().any(|&b| b != 0)
            {
                return Err(w::INVALID);
            }
            if app_catalog::is_browser(caller){
                let(id,generation)=b.create_new(name)?;
                if !(3..15).contains(&id)||generation==0{return Err(w::IO)}
                s.created=Some((id,generation));
                sex_pdx::serial_record!("[files.browser.import.created] target={} object={} identity={} create_only=1",caller,id,generation);
                return Ok(id as u32);
            }
            return b.create(name).map(|id| id as u32);
        }
        if app_catalog::is_browser(caller){
            if !matches!(r.kind,w::INFO|w::BEGIN_SAVE|w::WRITE|w::COMMIT|w::END){return Err(w::DENIED)}
            let Some((object,generation))=s.created else{return Err(w::DENIED)};
            if r.object!=object{return Err(w::DENIED)}
            if b.identity(object)!=Ok(generation){return Err(w::STALE)}
            if r.kind==w::END{
                if r.offset!=0||r.a!=0||r.b!=0{return Err(w::INVALID)}
                s.clear();return Ok(0);
            }
        }
        if r.object < 2 || r.object >= 15 {
            return Err(w::DENIED);
        }
        if r.kind == w::INFO {
            if s.bound || r.offset != 0 || r.a != 0 || r.b != 0 {
                return Err(w::BUSY);
            }
            return b.identity(r.object);
        }
        if r.kind == w::LOAD {
            if s.bound || r.offset != 0 || r.b > w::SNAPSHOT_MODE {
                return Err(w::BUSY);
            }
            let generation = b.identity(r.object)?;
            if r.a != generation as u64 {
                return Err(w::STALE);
            }
            b.clear_load(caller)?;
            s.clear();
            s.object = r.object;
            s.generation = generation;
            s.loading=Some(r);
            return Ok(0);
        }
        if r.kind == w::BEGIN_SAVE {
            if s.bound || r.offset != 0 || r.b > w::MAX as u64 {
                return Err(w::INVALID);
            }
            let generation = b.identity(r.object)?;
            if r.a != generation as u64 {
                return Err(w::STALE);
            }
            let created=s.created;
            s.clear();s.created=created;
            s.len = r.b as usize;
            s.name = b.name(r.object)?;
            s.object = r.object;
            s.generation = generation;
            s.bound = true;
            s.saving = true;
            return Ok(0);
        }
        if !s.bound || s.object != r.object || b.identity(r.object)? != s.generation {
            return Err(w::STALE);
        }
        match r.kind {
            w::WRITE => {
                let off = r.offset as usize;
                if !s.saving || off != s.written || off >= s.len {
                    return Err(w::INVALID);
                }
                let n = (s.len - off).min(16);
                let mut raw = [0; 16];
                raw[..8].copy_from_slice(&r.a.to_le_bytes());
                raw[8..].copy_from_slice(&r.b.to_le_bytes());
                if raw[n..].iter().any(|&b| b != 0) {
                    return Err(w::INVALID);
                }
                s.bytes[off..off + n].copy_from_slice(&raw[..n]);
                s.written += n;
                Ok(n as u32)
            }
            w::COMMIT => {
                if !s.saving
                    || s.written != s.len
                    || r.offset != 0
                    || r.b != 0
                    || w::hash(&s.bytes[..s.len]) != r.a
                {
                    return Err(w::INVALID);
                }
                s.commit = Some(r);
                Ok(0)
            }
            w::READ => {
                if s.saving {
                    return Err(w::DENIED);
                }
                let off = r.offset as usize;
                let n = r.a as usize;
                if n == 0 || n > 4 || off.checked_add(n).map_or(true, |end| end > s.len) || r.b != 0
                {
                    return Err(w::INVALID);
                }
                let mut v = [0; 4];
                v[..n].copy_from_slice(&s.bytes[off..off + n]);
                Ok(u32::from_le_bytes(v))
            }
            w::NAME => {
                let off = r.offset as usize;
                if off % 4 != 0 || off >= 16 || r.a != 0 || r.b != 0 {
                    return Err(w::INVALID);
                }
                Ok(u32::from_le_bytes(s.name[off..off + 4].try_into().unwrap()))
            }
            w::END => {
                if r.offset != 0 || r.a != 0 || r.b != 0 {
                    return Err(w::INVALID);
                }
                b.clear_load(caller)?;
                s.clear();
                Ok(0)
            }
            _ => Err(w::INVALID),
        }
    }
    pub fn handle(
        &mut self,
        caller: u32,
        live: bool,
        r: w::Request,
        b: &mut impl Backend,
    ) -> Option<u64> {
        let Some(i) = Self::index(caller) else {
            return Some(w::reply(r.seq, Err(w::DENIED)));
        };
        let s = &mut self.sessions[i];
        if s.dead || !live {
            return Some(w::reply(r.seq, Err(w::DENIED)));
        }
        // A queued duplicate consumes neither another receipt nor another
        // backend effect, including a COMMIT whose completion is not ready.
        if s.pending[..s.pending_len]
            .iter()
            .flatten()
            .any(|entry| entry.request == r)
        {
            return None;
        }
        let fresh_abort = r.kind == w::ABORT
            && r.object == 0
            && r.offset == 0
            && r.a == 0
            && r.b == 0
            && s.last.map_or(true, |old| r.seq > old.seq);
        if fresh_abort {
            // Terminal cancellation outranks saturation. Already delivered old
            // replies remain harmless because the client fences their sequence.
            s.pending = [None; RECEIPTS];
            s.pending_len = 0;
        } else if s.commit.is_some() || s.loading.is_some() || s.pending_len == RECEIPTS {
            // Beyond the four-request contract: reject before any effect.
            // This immediate rejection is not a retained accepted receipt.
            return Some(w::reply(r.seq, Err(w::BUSY)));
        }
        let slot = s.pending_len;
        s.pending[slot] = Some(Receipt {
            request: r,
            value: None,
        });
        s.pending_len += 1;
        let v = if let Some(old) = s.last {
            if old == r {
                s.receipt
            } else if r.seq <= old.seq {
                w::reply(r.seq, Err(w::STALE))
            } else {
                w::reply(r.seq, Self::execute(s, caller, r, b))
            }
        } else {
            w::reply(r.seq, Self::execute(s, caller, r, b))
        };
        if s.last.map_or(true, |old| r.seq > old.seq) {
            s.last = Some(r);
            s.receipt = v;
        }
        if s.commit.is_none() && s.loading.is_none() {
            s.pending[slot].as_mut().unwrap().value = Some(v);
        }
        None
    }
}
