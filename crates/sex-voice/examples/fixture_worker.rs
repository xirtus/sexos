//! Runnable contract fixture: cargo run --manifest-path crates/sex-voice/Cargo.toml --example fixture_worker
//! This models a local worker exchange without networking or model weights.
use sex_voice::{decode, encode, JobState, Message, Operation, ResultAsset, Status, Submit, MAX_FRAME};

// 8 kHz, mono, 16-bit PCM WAV containing four deterministic samples.
const WAV: [u8; 52] = [
    82, 73, 70, 70, 44, 0, 0, 0, 87, 65, 86, 69, 102, 109, 116, 32,
    16, 0, 0, 0, 1, 0, 1, 0, 64, 31, 0, 0, 128, 62, 0, 0,
    2, 0, 16, 0, 100, 97, 116, 97, 8, 0, 0, 0, 0, 0, 232, 3, 24, 252, 0, 0,
];
// SHA-256 of WAV above, representing content-addressed asset 77 in this fixture.
const WAV_SHA256: [u8; 32] = [
    0x61, 0x54, 0xe1, 0xa5, 0x10, 0x5d, 0x7b, 0x8b,
    0x35, 0xa7, 0xac, 0x22, 0x8d, 0x50, 0x90, 0x7c,
    0x2f, 0xed, 0x58, 0xea, 0x63, 0x57, 0x73, 0x91,
    0xb3, 0x49, 0x57, 0xcc, 0x53, 0x89, 0x66, 0xce,
];

struct Worker { job_id: u64, polls: u8, cancelled: bool }
impl Worker {
    fn new() -> Self { Self { job_id: 0, polls: 0, cancelled: false } }

    fn handle(&mut self, request_frame: &[u8], response: &mut [u8]) -> usize {
        let reply = match decode(request_frame).expect("valid fixture request") {
            Message::Submit { job_id, data } => {
                assert_eq!(data.operation, Operation::Synthesize);
                assert_eq!(self.job_id, 0);
                self.job_id = job_id;
                Message::Status { job_id, data: Status { state: JobState::Queued, progress_bps: 0, error_code: 0 } }
            }
            Message::Cancel { job_id } if job_id == self.job_id => {
                self.cancelled = true;
                Message::Status { job_id, data: Status { state: JobState::Cancelled, progress_bps: 0, error_code: 0 } }
            }
            Message::GetStatus { job_id } if job_id == self.job_id && self.cancelled => {
                Message::Status { job_id, data: Status { state: JobState::Cancelled, progress_bps: 0, error_code: 0 } }
            }
            Message::GetStatus { job_id } if job_id == self.job_id => {
                self.polls += 1;
                if self.polls == 1 {
                    Message::Status { job_id, data: Status { state: JobState::Running, progress_bps: 5_000, error_code: 0 } }
                } else {
                    Message::Result { job_id, data: ResultAsset { asset_id: 77, sample_rate: 8_000,
                        channels: 1, frames: 4, content_hash: WAV_SHA256, seed: 42, model: "fixture-v1" } }
                }
            }
            _ => panic!("unexpected fixture request"),
        };
        encode(reply, response).expect("valid fixture response")
    }
}

fn exchange<'a>(worker: &mut Worker, request: Message<'_>, response: &'a mut [u8; MAX_FRAME]) -> Message<'a> {
    let mut outgoing = [0; MAX_FRAME];
    let n = encode(request, &mut outgoing).expect("encode request");
    let used = worker.handle(&outgoing[..n], response);
    decode(&response[..used]).expect("decode response")
}

fn main() {
    let mut worker = Worker::new();
    let mut response = [0; MAX_FRAME];
    let submit = Message::Submit { job_id: 11, data: Submit { operation: Operation::Synthesize,
        seed: 42, voice_id: 9, reference_asset_id: 0, max_frames: 8_000,
        language: "en", model: "fixture-v1", text: "Hello" } };
    assert!(matches!(exchange(&mut worker, submit, &mut response),
        Message::Status { data: Status { state: JobState::Queued, .. }, .. }));
    assert!(matches!(exchange(&mut worker, Message::GetStatus { job_id: 11 }, &mut response),
        Message::Status { data: Status { state: JobState::Running, .. }, .. }));
    let result = exchange(&mut worker, Message::GetStatus { job_id: 11 }, &mut response);
    match result {
        Message::Result { data, .. } => {
            assert_eq!(data.frames, 4);
            assert_eq!(data.content_hash, WAV_SHA256);
            assert_eq!(WAV.len(), 44 + data.frames as usize * 2);
        }
        _ => panic!("expected fixture audio result"),
    }

    let mut cancelled = Worker::new();
    let cancel_submit = Message::Submit { job_id: 12, data: Submit { operation: Operation::Synthesize,
        seed: 42, voice_id: 9, reference_asset_id: 0, max_frames: 8_000,
        language: "en", model: "fixture-v1", text: "Stop" } };
    exchange(&mut cancelled, cancel_submit, &mut response);
    assert!(matches!(exchange(&mut cancelled, Message::Cancel { job_id: 12 }, &mut response),
        Message::Status { data: Status { state: JobState::Cancelled, .. }, .. }));
    assert!(matches!(exchange(&mut cancelled, Message::GetStatus { job_id: 12 }, &mut response),
        Message::Status { data: Status { state: JobState::Cancelled, .. }, .. }));

    let mut malformed = [0; MAX_FRAME];
    let n = encode(Message::GetStatus { job_id: 11 }, &mut malformed).unwrap();
    malformed[4] = 99;
    assert!(decode(&malformed[..n]).is_err());
    println!("voice fixture: submit, status, result, cancellation, malformed frame passed");
}
