// SPDX-License-Identifier: GPL-3.0-only
//! `tether-bridge`: Host-side Antigravity ACP + SexOS MCP bridge.
//!
//! Usage:
//!   tether-bridge serve [--port 7178] [--control-port 7179] [--pin 123456 | --no-auth] [--acp-cmd "agy acp" | --mock-acp]
//!   tether-bridge mcp [--control-port 7179]
//!   tether-bridge status [--control-port 7179]
//!   tether-bridge call <method> [params_json] [--control-port 7179]

mod bridge;
mod mcp;

use std::env;
use std::process;
use tether_proto::frame::{DEFAULT_CONTROL_PORT, DEFAULT_PORT};

fn print_help() {
    eprintln!(
        "tether-bridge v0.1.0 — SexOS Tether v2 Host Bridge (ACP + MCP)\n\
         \n\
         Subcommands:\n\
           serve        Listen for SexOS `apps/tether` connections, bridge `agy acp`, and serve local MCP/CLI control\n\
           mcp          Run MCP JSON-RPC 2.0 over stdio for Antigravity and fanned-out subagents\n\
           status       Query connected SexOS status from running `serve` daemon\n\
           call         Invoke a typed SexOS RPC method (e.g. `call sysinfo` or `call exec '{{\"command\":\"hw\"}}'`)\n"
    );
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        print_help();
        return;
    }

    let mut port = DEFAULT_PORT;
    let mut control_port = DEFAULT_CONTROL_PORT;
    let mut pin: Option<String> = None;
    let mut no_auth = false;
    let mut mock_acp = false;
    let mut acp_cmd = vec![String::from("agy"), String::from("acp")];
    let mut positional = Vec::new();
    let mut agent = String::from("antigravity");
    let mut claude_cmd: Vec<String> = Vec::new();
    let mut workdir: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" if i + 1 < args.len() => {
                port = args[i + 1].parse().unwrap_or(DEFAULT_PORT);
                i += 2;
            }
            "--control-port" if i + 1 < args.len() => {
                control_port = args[i + 1].parse().unwrap_or(DEFAULT_CONTROL_PORT);
                i += 2;
            }
            "--pin" if i + 1 < args.len() => {
                pin = Some(args[i + 1].clone());
                i += 2;
            }
            "--no-auth" => {
                no_auth = true;
                i += 1;
            }
            "--mock-acp" => {
                mock_acp = true;
                i += 1;
            }
            "--agent" if i + 1 < args.len() => {
                agent = args[i + 1].to_ascii_lowercase();
                i += 2;
            }
            "--claude-cmd" if i + 1 < args.len() => {
                claude_cmd = args[i + 1].split_whitespace().map(String::from).collect();
                i += 2;
            }
            "--workdir" if i + 1 < args.len() => {
                workdir = Some(args[i + 1].clone());
                i += 2;
            }
            "--acp-cmd" if i + 1 < args.len() => {
                acp_cmd = args[i + 1].split_whitespace().map(String::from).collect();
                i += 2;
            }
            other => {
                positional.push(other.to_string());
                i += 1;
            }
        }
    }

    let control_addr = format!("127.0.0.1:{}", control_port);
    let res = match args[0].as_str() {
        "serve" => {
            let final_pin = if no_auth {
                None
            } else {
                Some(pin.unwrap_or_else(bridge::generate_pin))
            };
            bridge::run_serve(bridge::BridgeConfig {
                listen_addr: format!("0.0.0.0:{}", port),
                control_addr,
                pin: final_pin,
                acp_cmd,
                mock_acp,
                agent,
                claude_cmd,
                workdir,
            })
        }
        "mcp" => mcp::run_mcp_stdio(&control_addr),
        "status" => match bridge::control_call(&control_addr, "{\"method\":\"status\"}") {
            Ok(out) => {
                println!("{}", out);
                Ok(())
            }
            Err(e) => Err(e),
        },
        "call" => {
            let method = positional.first().map(String::as_str).unwrap_or("status");
            let params = positional.get(1).map(String::as_str).unwrap_or("{}");
            let req = format!(
                "{{\"method\":\"{}\",\"caller\":\"cli\",\"params\":{}}}",
                method, params
            );
            match bridge::control_call(&control_addr, &req) {
                Ok(out) => {
                    println!("{}", out);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        other => Err(format!("unknown subcommand '{}'", other)),
    };

    if let Err(e) = res {
        eprintln!("tether-bridge: error: {}", e);
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::bridge::{control_call, start_test_bridge};
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::thread;
    use std::time::Duration;
    use tether_proto::acp::{acp_initialize, acp_session_prompt, parse_acp_incoming, AcpIncoming};
    use tether_proto::auth::{compute_auth_tag, verify_auth_tag};
    use tether_proto::frame::{Frame, FrameDecoder, FrameKind, MAGIC};
    use tether_proto::rpc::{json_get_str, rpc_ok_json, CapabilityMode};

    #[test]
    fn end_to_end_pin_auth_acp_rpc_and_kill_switch() {
        let pin = String::from("482910");
        let (listen_addr, ctrl_addr, _state) = start_test_bridge(Some(pin.clone()));

        // Simulated SexOS `apps/tether` client connecting to host bridge
        let mut client = TcpStream::connect(&listen_addr).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut dec = FrameDecoder::new(true);

        // Send magic + Hello (capability = Dev)
        let mut hello = Vec::new();
        hello.extend_from_slice(&MAGIC);
        Frame::control(
            FrameKind::Hello,
            format!("{{\"role\":\"sexos-tether\",\"capability\":{}}}", CapabilityMode::Dev as u8)
                .into_bytes(),
        )
        .encode(&mut hello);
        client.write_all(&hello).unwrap();

        let read_one = |s: &mut TcpStream, d: &mut FrameDecoder| -> Frame {
            let mut buf = [0u8; 4096];
            loop {
                if let Some(f) = d.next_frame().unwrap() {
                    return f;
                }
                let n = s.read(&mut buf).unwrap();
                assert!(n > 0);
                d.push(&buf[..n]).unwrap();
            }
        };

        // Host Hello -> AuthChallenge -> AuthResponse -> AuthOk
        let host_hello = read_one(&mut client, &mut dec);
        assert_eq!(host_hello.kind, FrameKind::Hello);
        let chal = read_one(&mut client, &mut dec);
        assert_eq!(chal.kind, FrameKind::AuthChallenge);
        let mut nonce = [0u8; 32];
        nonce.copy_from_slice(&chal.payload);
        let tag = compute_auth_tag(&pin, &nonce, b"sexos-client");
        client
            .write_all(&Frame::control(FrameKind::AuthResponse, tag.to_vec()).to_bytes())
            .unwrap();
        let ok = read_one(&mut client, &mut dec);
        assert_eq!(ok.kind, FrameKind::AuthOk);
        let mut host_tag = [0u8; 32];
        host_tag.copy_from_slice(&ok.payload);
        assert!(verify_auth_tag(&pin, &nonce, b"host-bridge", &host_tag));

        // 1. ACP roundtrip over Stream 0
        let init_msg = acp_initialize(1, "SexOS Tether", "0.1.0");
        client.write_all(&Frame::acp(init_msg.as_bytes()).to_bytes()).unwrap();
        let init_reply = read_one(&mut client, &mut dec);
        assert_eq!(init_reply.kind, FrameKind::AcpJson);
        assert!(matches!(
            parse_acp_incoming(&String::from_utf8(init_reply.payload).unwrap()),
            AcpIncoming::Initialized { id: 1, .. }
        ));

        let prompt_msg = acp_session_prompt(2, "sexos-mock-1", "Inspect Wi-Fi");
        client.write_all(&Frame::acp(prompt_msg.as_bytes()).to_bytes()).unwrap();
        let chunk = read_one(&mut client, &mut dec);
        let done = read_one(&mut client, &mut dec);
        assert!(matches!(
            parse_acp_incoming(&String::from_utf8(chunk.payload).unwrap()),
            AcpIncoming::AgentMessageChunk { .. }
        ));
        assert!(matches!(
            parse_acp_incoming(&String::from_utf8(done.payload).unwrap()),
            AcpIncoming::PromptDone { id: 2, .. }
        ));

        // 2. Host MCP/CLI RPC -> SexOS -> Host reply
        let ctrl_addr_clone = ctrl_addr.clone();
        let rpc_thread = thread::spawn(move || {
            control_call(&ctrl_addr_clone, "{\"method\":\"sysinfo\",\"caller\":\"mcp\"}").unwrap()
        });

        let rpc_req = read_one(&mut client, &mut dec);
        assert_eq!(rpc_req.kind, FrameKind::RpcRequest);
        let req_str = String::from_utf8(rpc_req.payload).unwrap();
        assert_eq!(json_get_str(&req_str, "method").as_deref(), Some("sysinfo"));

        let rpc_reply = rpc_ok_json(1, "{\"os\":\"SexOS\",\"pds\":42}");
        client
            .write_all(
                &Frame::new(
                    rpc_req.stream_id,
                    FrameKind::RpcResponse,
                    0,
                    rpc_reply.into_bytes(),
                )
                .to_bytes(),
            )
            .unwrap();
        let got_rpc = rpc_thread.join().unwrap();
        assert!(got_rpc.contains("\"os\":\"SexOS\""));

        // 3. Capability enforcement: Dev mode blocks FullControl `input_inject`
        let denied = control_call(
            &ctrl_addr,
            "{\"method\":\"input_inject\",\"caller\":\"mcp\",\"params\":{\"kind\":\"click\"}}",
        )
        .unwrap();
        assert!(denied.contains("capability_denied"));

        // 4. Kill switch blocks subsequent RPCs
        client
            .write_all(&Frame::control(FrameKind::KillSwitch, vec![1]).to_bytes())
            .unwrap();
        thread::sleep(Duration::from_millis(50));
        let blocked = control_call(&ctrl_addr, "{\"method\":\"sysinfo\",\"caller\":\"mcp\"}").unwrap();
        assert!(blocked.contains("kill_switch"));
    }

    #[test]
    fn binary_fs_write_hotswap_heartbeat_and_clean_preemption() {
        use tether_proto::auth::fnv64;
        use tether_proto::frame::{HEARTBEAT_PING, HEARTBEAT_PONG};
        use tether_proto::rpc::{json_get_bool, json_get_u64};

        let (listen_addr, ctrl_addr, state) = start_test_bridge(None);

        let connect_client = |cap: CapabilityMode| -> (TcpStream, FrameDecoder) {
            let mut s = TcpStream::connect(&listen_addr).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut dec = FrameDecoder::new(true);
            let mut hello = Vec::new();
            hello.extend_from_slice(&MAGIC);
            Frame::control(
                FrameKind::Hello,
                format!("{{\"role\":\"sexos-tether\",\"capability\":{}}}", cap as u8).into_bytes(),
            )
            .encode(&mut hello);
            s.write_all(&hello).unwrap();

            let mut buf = [0u8; 4096];
            let mut got_ok = false;
            while !got_ok {
                let n = s.read(&mut buf).unwrap();
                assert!(n > 0);
                dec.push(&buf[..n]).unwrap();
                while let Some(f) = dec.next_frame().unwrap() {
                    if f.kind == FrameKind::AuthOk {
                        got_ok = true;
                        break;
                    }
                }
            }
            (s, dec)
        };

        let read_one = |s: &mut TcpStream, d: &mut FrameDecoder| -> Frame {
            let mut buf = [0u8; 16384];
            loop {
                if let Some(f) = d.next_frame().unwrap() {
                    return f;
                }
                let n = s.read(&mut buf).unwrap();
                assert!(n > 0);
                d.push(&buf[..n]).unwrap();
            }
        };

        // Session 1 connects, then Session 2 preempts it; Session 1 must receive EOF/shutdown cleanly.
        let (mut old_client, _old_dec) = connect_client(CapabilityMode::Observe);
        let (mut client, mut dec) = connect_client(CapabilityMode::Dev);
        let mut eof_buf = [0u8; 64];
        old_client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let old_res = old_client.read(&mut eof_buf);
        assert!(matches!(old_res, Ok(0) | Err(_)), "preempted session should close");
        drop(old_client);

        // 1. Heartbeat Ping -> Pong (and verify Pong is NOT echoed back)
        client
            .write_all(&Frame::heartbeat_ping(0x1234).to_bytes())
            .unwrap();
        let pong = read_one(&mut client, &mut dec);
        assert_eq!(pong.kind, FrameKind::Heartbeat);
        assert_eq!(pong.payload[0], HEARTBEAT_PONG);
        assert_eq!(&pong.payload[1..], &0x1234u64.to_le_bytes());

        // Sending a Pong must NOT trigger another Heartbeat from host
        client
            .write_all(&Frame::heartbeat_pong(&[HEARTBEAT_PING, 9, 9]).to_bytes())
            .unwrap();

        // 2. Binary fs_write via local_path (multi-chunk > 64 KiB with null bytes)
        let tmp_bin = std::env::temp_dir().join(format!(
            "tether-test-bin-{}.elf",
            std::process::id()
        ));
        let mut payload = vec![0x7f, b'E', b'L', b'F'];
        for i in 0..(70 * 1024usize) {
            payload.push((i % 251) as u8);
        }
        std::fs::write(&tmp_bin, &payload).unwrap();
        let expected_hash = format!("{:016x}", fnv64(&payload));

        let ctrl_c = ctrl_addr.clone();
        let lp = tmp_bin.to_string_lossy().into_owned();
        let write_thread = thread::spawn(move || {
            let req = format!(
                "{{\"method\":\"fs_write\",\"caller\":\"mcp\",\"params\":{{\"path\":\"/bin/splicy\",\"local_path\":\"{}\"}}}}",
                tether_proto::rpc::json_escape(&lp)
            );
            control_call(&ctrl_c, &req).unwrap()
        });

        let req_frame = read_one(&mut client, &mut dec);
        assert_eq!(req_frame.kind, FrameKind::RpcRequest);
        let req_str = String::from_utf8(req_frame.payload).unwrap();
        assert_eq!(json_get_str(&req_str, "method").as_deref(), Some("fs_write"));
        assert_eq!(json_get_bool(&req_str, "stream"), Some(true));
        assert_eq!(json_get_u64(&req_str, "size"), Some(payload.len() as u64));
        assert_eq!(json_get_str(&req_str, "fnv64").as_deref(), Some(expected_hash.as_str()));

        let mut received = Vec::new();
        let mut chunk_count = 0usize;
        loop {
            let f = read_one(&mut client, &mut dec);
            assert_eq!(f.stream_id, req_frame.stream_id);
            match f.kind {
                FrameKind::RpcStreamChunk => {
                    chunk_count += 1;
                    received.extend_from_slice(&f.payload);
                }
                FrameKind::RpcStreamEnd => break,
                other => panic!("unexpected frame during upload: {:?}", other),
            }
        }
        assert!(chunk_count >= 3, "expected multi-chunk upload");
        assert_eq!(received, payload);

        let reply = rpc_ok_json(1, &format!("{{\"written\":{}}}", received.len()));
        client
            .write_all(
                &Frame::new(req_frame.stream_id, FrameKind::RpcResponse, 0, reply.into_bytes())
                    .to_bytes(),
            )
            .unwrap();
        let write_out = write_thread.join().unwrap();
        assert!(write_out.contains(&format!("\"written\":{}", payload.len())));

        // 3. Binary fs_write via base64
        let ctrl_c = ctrl_addr.clone();
        let b64_thread = thread::spawn(move || {
            let req = "{\"method\":\"fs_write\",\"caller\":\"mcp\",\"params\":{\"path\":\"/tmp/b64.bin\",\"base64\":\"f0VMRgAA\"}}";
            control_call(&ctrl_c, req).unwrap()
        });
        let b64_req = read_one(&mut client, &mut dec);
        assert_eq!(b64_req.kind, FrameKind::RpcRequest);
        let b64_chunk = read_one(&mut client, &mut dec);
        assert_eq!(b64_chunk.kind, FrameKind::RpcStreamChunk);
        assert_eq!(b64_chunk.payload, b"\x7fELF\x00\x00");
        let b64_end = read_one(&mut client, &mut dec);
        assert_eq!(b64_end.kind, FrameKind::RpcStreamEnd);
        client
            .write_all(
                &Frame::new(
                    b64_req.stream_id,
                    FrameKind::RpcResponse,
                    0,
                    rpc_ok_json(2, "{\"written\":6}").into_bytes(),
                )
                .to_bytes(),
            )
            .unwrap();
        assert!(b64_thread.join().unwrap().contains("\"written\":6"));

        // 4. Hotswap via local ELF path streams chunks + RpcStreamEnd
        let ctrl_c = ctrl_addr.clone();
        let lp = tmp_bin.to_string_lossy().into_owned();
        let hs_thread = thread::spawn(move || {
            let req = format!(
                "{{\"method\":\"hotswap\",\"caller\":\"mcp\",\"params\":{{\"server\":\"sexbell\",\"elf_path\":\"{}\"}}}}",
                tether_proto::rpc::json_escape(&lp)
            );
            control_call(&ctrl_c, &req).unwrap()
        });
        let hs_req = read_one(&mut client, &mut dec);
        assert_eq!(hs_req.kind, FrameKind::RpcRequest);
        let hs_str = String::from_utf8(hs_req.payload).unwrap();
        assert_eq!(json_get_str(&hs_str, "method").as_deref(), Some("hotswap"));
        assert_eq!(json_get_str(&hs_str, "server").as_deref(), Some("sexbell"));
        assert_eq!(json_get_bool(&hs_str, "stream"), Some(true));
        let mut hs_bytes = Vec::new();
        loop {
            let f = read_one(&mut client, &mut dec);
            match f.kind {
                FrameKind::RpcStreamChunk => hs_bytes.extend_from_slice(&f.payload),
                FrameKind::RpcStreamEnd => break,
                other => panic!("unexpected hotswap frame: {:?}", other),
            }
        }
        assert_eq!(hs_bytes, payload);
        client
            .write_all(
                &Frame::new(
                    hs_req.stream_id,
                    FrameKind::RpcResponse,
                    0,
                    rpc_ok_json(3, "{\"server\":\"sexbell\",\"pd\":10}").into_bytes(),
                )
                .to_bytes(),
            )
            .unwrap();
        assert!(hs_thread.join().unwrap().contains("\"pd\":10"));

        let _ = std::fs::remove_file(&tmp_bin);
        drop(client);
        thread::sleep(Duration::from_millis(100));
        assert!(!state.connected.load(std::sync::atomic::Ordering::SeqCst));
    }
}
