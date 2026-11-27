// SPDX-License-Identifier: GPL-3.0-only
//! Tether v2 protocol crate shared between SexOS `apps/tether` (`no_std`) and host `tether-bridge` (`std`).
#![no_std]

extern crate alloc;

pub mod acp;
pub mod auth;
pub mod frame;
pub mod rpc;

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use crate::acp::{acp_initialize, acp_session_prompt, parse_acp_incoming, AcpIncoming};
    use crate::auth::{compute_auth_tag, fnv64, sha256, verify_auth_tag};
    use crate::frame::{
        DecodeError, Frame, FrameDecoder, FrameKind, MAGIC, STREAM_ACP, STREAM_RPC_START,
    };
    use crate::rpc::{json_escape, json_get_bool, json_get_str, json_get_u64, CapabilityMode, RpcMethod};

    #[test]
    fn frame_roundtrip_and_partial_reads() {
        let f1 = Frame::acp(br#"{"jsonrpc":"2.0","method":"ping"}"#);
        let f2 = Frame::new(STREAM_RPC_START, FrameKind::RpcRequest, 0, b"hello".to_vec());

        let mut wire = Vec::new();
        wire.extend_from_slice(&MAGIC);
        f1.encode(&mut wire);
        f2.encode(&mut wire);

        let mut dec = FrameDecoder::new(true);
        let mut got = Vec::new();
        for chunk in wire.chunks(3) {
            dec.push(chunk).unwrap();
            while let Some(frame) = dec.next_frame().unwrap() {
                got.push(frame);
            }
        }
        assert_eq!(got, vec![f1, f2]);
        assert_eq!(dec.buffered_len(), 0);
    }

    #[test]
    fn frame_rejects_bad_magic_and_oversized_payload() {
        let mut dec = FrameDecoder::new(true);
        dec.push(b"NOPE").unwrap();
        assert_eq!(dec.next_frame(), Err(DecodeError::BadMagic));

        let mut dec2 = FrameDecoder::with_max_payload(false, 16);
        let big = Frame::new(STREAM_ACP, FrameKind::AcpJson, 0, vec![b'a'; 32]);
        dec2.push(&big.to_bytes()).unwrap();
        assert_eq!(dec2.next_frame(), Err(DecodeError::PayloadTooLarge(32)));
    }

    #[test]
    fn sha256_known_vector_and_pin_auth() {
        let digest = sha256(b"abc");
        assert_eq!(
            digest,
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea,
                0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
                0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c,
                0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
            ]
        );

        let nonce = [0x42u8; 32];
        let tag = compute_auth_tag("482910", &nonce, b"sexos-client");
        assert!(verify_auth_tag("482910", &nonce, b"sexos-client", &tag));
        assert!(!verify_auth_tag("000000", &nonce, b"sexos-client", &tag));
        assert!(!verify_auth_tag("482910", &nonce, b"host-bridge", &tag));
        assert_eq!(fnv64(b""), 0xcbf2_9ce4_8422_2325);
    }

    #[test]
    fn capability_tier_enforcement() {
        assert!(CapabilityMode::Observe.allows(RpcMethod::SysInfo));
        assert!(CapabilityMode::Observe.allows(RpcMethod::Screenshot));
        assert!(!CapabilityMode::Observe.allows(RpcMethod::Exec));
        assert!(!CapabilityMode::Observe.allows(RpcMethod::Hotswap));
        assert!(CapabilityMode::Dev.allows(RpcMethod::Hotswap));
        assert!(!CapabilityMode::Dev.allows(RpcMethod::InputInject));
        assert!(CapabilityMode::FullControl.allows(RpcMethod::InputInject));
        assert!(CapabilityMode::FullControl.allows(RpcMethod::OtaPush));
        assert!(!RpcMethod::Screenshot.is_mutating());
        assert!(RpcMethod::Hotswap.is_mutating());
    }

    #[test]
    fn json_and_acp_parsing() {
        let init = acp_initialize(1, "SexOS Tether", "0.1.0");
        assert_eq!(json_get_u64(&init, "id"), Some(1));
        assert_eq!(json_get_str(&init, "method").as_deref(), Some("initialize"));
        assert_eq!(json_get_bool(&init, "terminal"), Some(true));

        let escaped = json_escape("line1\n\"quoted\"\\end");
        assert_eq!(escaped, "line1\\n\\\"quoted\\\"\\\\end");

        let chunk = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Hello from Antigravity!"}}}}"#;
        assert_eq!(
            parse_acp_incoming(chunk),
            AcpIncoming::AgentMessageChunk {
                text: alloc::string::String::from("Hello from Antigravity!")
            }
        );

        let perm = r#"{"jsonrpc":"2.0","id":7,"method":"session/request_permission","params":{"toolCall":{"toolCallId":"tc-1","title":"Hotswap sexdisplay"}}}"#;
        assert_eq!(
            parse_acp_incoming(perm),
            AcpIncoming::PermissionRequest {
                id: 7,
                title: alloc::string::String::from("Hotswap sexdisplay"),
                tool_call_id: alloc::string::String::from("tc-1"),
            }
        );

        let prompt = acp_session_prompt(2, "sess-1", "Check Wi-Fi");
        assert!(prompt.contains("\"session/prompt\""));
    }

    #[test]
    fn heartbeat_ping_pong_and_base64_and_fnv_update() {
        use crate::auth::{fnv64_update, FNV64_OFFSET};
        use crate::frame::{HEARTBEAT_PING, HEARTBEAT_PONG};
        use crate::rpc::decode_base64;

        let ping = Frame::heartbeat_ping(0x1122_3344_5566_7788);
        assert_eq!(ping.kind, FrameKind::Heartbeat);
        assert_eq!(ping.payload[0], HEARTBEAT_PING);
        assert_eq!(ping.payload.len(), 9);

        let pong = Frame::heartbeat_pong(&ping.payload);
        assert_eq!(pong.kind, FrameKind::Heartbeat);
        assert_eq!(pong.payload[0], HEARTBEAT_PONG);
        assert_eq!(&pong.payload[1..], &ping.payload[1..]);

        assert_eq!(decode_base64("").unwrap(), Vec::<u8>::new());
        assert_eq!(decode_base64("Zg==").unwrap(), b"f");
        assert_eq!(decode_base64("Zm8=").unwrap(), b"fo");
        assert_eq!(decode_base64("Zm9v\nYmFy").unwrap(), b"foobar");
        assert_eq!(decode_base64("f0VMRg==").unwrap(), b"\x7fELF");
        assert!(decode_base64("!!!").is_err());
        assert!(decode_base64("Zg=").is_err());

        let h = fnv64_update(FNV64_OFFSET, b"foo");
        let h = fnv64_update(h, b"bar");
        assert_eq!(h, fnv64(b"foobar"));
    }
}
