use core::ops::Range;

use crate::{
    TlsError, alert::AlertDescription, common::decrypted_buffer_info::DecryptedBufferInfo,
    config::TlsCipherSuite, handshake::ServerHandshake, record::ServerRecord,
};

pub struct DecryptedReadHandler<'a> {
    pub source_buffer: Range<*const u8>,
    pub buffer_info: &'a mut DecryptedBufferInfo,
    pub is_open: &'a mut bool,
}

impl DecryptedReadHandler<'_> {
    pub fn handle<CipherSuite: TlsCipherSuite>(
        &mut self,
        record: ServerRecord<'_, CipherSuite>,
    ) -> Result<(), TlsError> {
        match record {
            ServerRecord::ApplicationData(data) => {
                let slice = data.data.as_slice();
                let slice_ptrs = slice.as_ptr_range();

                debug_assert!(
                    self.source_buffer.contains(&slice_ptrs.start)
                        && self.source_buffer.contains(&slice_ptrs.end)
                );

                let offset = unsafe {
                    // SAFETY: The assertion above ensures `slice` is a subslice of the read buffer.
                    // This, in turn, ensures we don't violate safety constraints of `offset_from`.

                    // TODO: We are only assuming here that the pointers are derived from the read
                    // buffer. While this is reasonable, and we don't do any pointer magic,
                    // it's not an invariant.
                    slice_ptrs.start.offset_from(self.source_buffer.start) as usize
                };

                self.buffer_info.offset = offset;
                self.buffer_info.len = slice.len();
                self.buffer_info.consumed = 0;
                Ok(())
            }
            ServerRecord::Alert(alert) => {
                if let AlertDescription::CloseNotify = alert.description {
                    *self.is_open = false;
                    Err(TlsError::ConnectionClosed)
                } else {
                    Err(TlsError::InternalError)
                }
            }
            ServerRecord::ChangeCipherSpec(_) => Err(TlsError::InternalError),
            ServerRecord::Handshake(ServerHandshake::NewSessionTicket(_)) => {
                // TODO: we should validate extensions and abort. We can do this automatically
                // as long as the connection is unsplit, however, split connections must be aborted
                // by the user.
                Ok(())
            }
            // SexOS: unsupported peer-controlled post-handshake messages are
            // protocol errors, never an application panic.
            ServerRecord::Handshake(_) => Err(TlsError::InvalidHandshake),
        }
    }
}

#[cfg(test)]
mod sexos_post_handshake_regressions {
    use super::*;
    use crate::{Aes128GcmSha256, record::RecordHeader};
    use sha2::{Digest, Sha256};

    #[test]
    fn parsed_unexpected_post_handshake_message_returns_error_without_panic() {
        // An actual parsed CertificateVerify handshake is inappropriate during
        // application-data reads. Its zero-length signature is parsing data;
        // the read handler must reject the record without attempting auth.
        let mut bytes = [15u8, 0, 0, 4, 4, 3, 0, 0];
        let header = RecordHeader::decode([22, 3, 3, 0, 8]).unwrap();
        let source_buffer = bytes.as_ptr_range();
        let mut hash = Sha256::new();
        let record = ServerRecord::<Aes128GcmSha256>::decode(header, &mut bytes, &mut hash).unwrap();
        assert!(matches!(record, ServerRecord::Handshake(_)));
        let mut info = DecryptedBufferInfo::default();
        let mut open = true;
        let mut handler = DecryptedReadHandler {source_buffer, buffer_info:&mut info, is_open:&mut open};
        assert!(matches!(handler.handle(record), Err(TlsError::InvalidHandshake)));
        assert_eq!(info.len, 0);
    }
    #[test]
    fn parsed_close_notify_is_connection_closed_and_clears_open_state() {
        let mut bytes = [1u8, 0];
        let header = RecordHeader::decode([21, 3, 3, 0, 2]).unwrap();
        let source_buffer = bytes.as_ptr_range();
        let mut hash = Sha256::new();
        let record = ServerRecord::<Aes128GcmSha256>::decode(header, &mut bytes, &mut hash).unwrap();
        let mut info = DecryptedBufferInfo::default();
        let mut open = true;
        let mut handler = DecryptedReadHandler {source_buffer, buffer_info:&mut info, is_open:&mut open};
        assert!(matches!(handler.handle(record), Err(TlsError::ConnectionClosed)));
        assert!(!open);
        assert_eq!(info.len, 0);
    }
}
