//! Bounded service-local custody of requests received during a block wait.
//! Authentic kernel exit hints have a separate slot per BOOT lifecycle entry;
//! they drain first, but authorize nothing until ServiceCustody attests them.
pub type Message = (u64, u32, u64, u64, u64);
const EMPTY: Message = (0, 0, 0, 0, 0);
const REQUESTS: usize = 8;
const EXITS: usize = sex_pdx::app_lifecycle::SLOTS.len();
pub struct Deferred {
    requests: [Message; REQUESTS],
    request_len: usize,
    exits: [Message; EXITS],
    exit_len: usize,
}
impl Deferred {
    pub const fn new() -> Self {
        Self {
            requests: [EMPTY; REQUESTS],
            request_len: 0,
            exits: [EMPTY; EXITS],
            exit_len: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.request_len + self.exit_len
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
            if self.request_len == REQUESTS {
                return false;
            }
            self.requests[self.request_len] = m;
            self.request_len += 1;
            true
        }
    }
    pub fn pop(&mut self) -> Option<Message> {
        let (queue, len) = if self.exit_len != 0 {
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
        Some(message)
    }
}
