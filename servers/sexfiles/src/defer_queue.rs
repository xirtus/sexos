//! Bounded service-local custody of requests received during a block wait.
//! Authentic kernel exit hints have a separate slot per BOOT lifecycle entry;
//! they drain first, but authorize nothing until ServiceCustody attests them.
pub type Message = (u64, u32, u64, u64, u64);
const EMPTY: Message = (0, 0, 0, 0, 0);
const REQUESTS: usize = 8;
const DOCUMENT_CLIENTS: usize = sex_pdx::app_catalog::DOCUMENT_TARGETS.len();
const TOTAL_REQUESTS: usize = REQUESTS + DOCUMENT_CLIENTS * (sex_pdx::document::WINDOW + 1);
const EXITS: usize = sex_pdx::app_lifecycle::SLOTS.len();
pub struct Deferred {
    requests: [Message; TOTAL_REQUESTS],
    request_len: usize,
    ordinary_len: usize,
    document_len: [usize; DOCUMENT_CLIENTS],
    cancellation: [bool; DOCUMENT_CLIENTS],
    exits: [Message; EXITS],
    exit_len: usize,
}
impl Deferred {
    pub const fn new() -> Self {
        Self {
            requests: [EMPTY; TOTAL_REQUESTS],
            request_len: 0,
            ordinary_len: 0,
            document_len: [0; DOCUMENT_CLIENTS],
            cancellation: [false; DOCUMENT_CLIENTS],
            exits: [EMPTY; EXITS],
            exit_len: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.request_len + self.exit_len
    }
    // Capacity classification only. The trampoline still performs exact
    // lifecycle admission before the document service can execute anything.
    fn document_client(m: Message) -> Option<usize> {
        if m.0 != sex_pdx::document::OP
            || sex_pdx::document::Request::decode(m.2, m.3, m.4).is_none()
        {
            return None;
        }
        sex_pdx::app_catalog::DOCUMENT_TARGETS.iter().position(|&target| target == m.1)
    }
    fn cancellation_client(m: Message) -> Option<usize> {
        let i = Self::document_client(m)?;
        let r = sex_pdx::document::Request::decode(m.2, m.3, m.4)?;
        (r.kind == sex_pdx::document::ABORT
            && r.object == 0
            && r.offset == 0
            && r.a == 0
            && r.b == 0)
            .then_some(i)
    }
    pub fn push(&mut self, m: Message) -> bool {
        if sex_pdx::app_lifecycle::ExitNotice::decode(m.0, m.1 as u64, [m.2, m.3, m.4]).is_ok() {
            if self.exits[..self.exit_len].contains(&m) {
                return true;
            }
            if self.exit_len == EXITS {
                return false;
            }
            self.exits[self.exit_len] = m;
            self.exit_len += 1;
            true
        } else {
            if let Some(i) = Self::cancellation_client(m) {
                if self.cancellation[i] {
                    let index = self.requests[..self.request_len]
                        .iter()
                        .position(|&old| Self::cancellation_client(old) == Some(i))
                        .unwrap();
                    let old = self.requests[index];
                    if old == m {
                        return true;
                    }
                    // Cancellation is idempotent. Retain the latest exact
                    // request at its arrival position, without stealing a data
                    // slot or reordering any remaining messages.
                    if m.2 & 0xFF_FFFF <= old.2 & 0xFF_FFFF {
                        return false;
                    }
                    self.requests
                        .copy_within(index + 1..self.request_len, index);
                    self.request_len -= 1;
                } else {
                    self.cancellation[i] = true;
                }
            } else if let Some(i) = Self::document_client(m) {
                if self.document_len[i] == sex_pdx::document::WINDOW {
                    return false;
                }
                self.document_len[i] += 1;
            } else {
                if self.ordinary_len == REQUESTS {
                    return false;
                }
                self.ordinary_len += 1;
            }
            self.requests[self.request_len] = m;
            self.request_len += 1;
            true
        }
    }
    pub fn pop(&mut self) -> Option<Message> {
        let terminal = self.exit_len != 0;
        let (queue, len) = if terminal {
            (&mut self.exits[..], &mut self.exit_len)
        } else {
            (&mut self.requests[..], &mut self.request_len)
        };
        if *len == 0 {
            return None;
        }
        let message = queue[0];
        queue.copy_within(1..*len, 0);
        *len -= 1;
        queue[*len] = EMPTY;
        if !terminal {
            if let Some(i) = Self::cancellation_client(message) {
                self.cancellation[i] = false;
            } else if let Some(i) = Self::document_client(message) {
                self.document_len[i] -= 1;
            } else {
                self.ordinary_len -= 1;
            }
        }
        Some(message)
    }
}
