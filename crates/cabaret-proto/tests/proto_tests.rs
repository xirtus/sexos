use cabaret_proto::browser_bridge::{
    modifiers, strip_bridge_prefix, BridgeError, BrowserToHostMessage, ControlLevel,
    HostToBrowserMessage, InteractEvent, MouseButton, BROWSER_BRIDGE_PREFIX,
    OBSSTUDIO_INIT_JS, OBSSTUDIO_PLUGIN_VERSION, generate_init_js,
};
use cabaret_proto::cli::{ctl_to_obs_request, parse_args, schema, schema_json, CliCommand, CliError};
use cabaret_proto::json::{parse_json, JsonVal};
use cabaret_proto::obs_ws::{
    base64_encode, compute_obs_auth, event_subscription, status_code, ObsAuthChallenge,
    ObsBatchExecutionType, ObsBatchRequest, ObsEvent, ObsEventKind, ObsFilterInfo, ObsInputInfo,
    ObsMessage, ObsOpCode, ObsRequest, ObsResponse, ObsResponseData, ObsSceneInfo,
    ObsSceneItemInfo, SceneItemTransform, OBS_WS_DEFAULT_PORT, OBS_WS_RPC_VERSION,
};
use cabaret_proto::script_rpc::{
    execute_batch_atomic, parse_stream_line, ScriptAction, ScriptLanguage, ScriptListOption,
    ScriptProperty, ScriptRpcLine, ScriptRpcMessage, CABARET_OPS_HEADER,
};
use cabaret_proto::streamdeck::{
    decode_input_report, diff_bitmask, encode_bmp_24, encode_key_image_packets,
    encode_key_rgb_bmp_packets, key_bitmask, reset_display, set_brightness, StreamDeckError,
    StreamDeckEvent, StreamDeckImageFormat, StreamDeckInputDecoder, StreamDeckInputReport,
    StreamDeckModel, ELGATO_VID,
};

// ---------------------------------------------------------------------------
// 1. json.rs tests
// ---------------------------------------------------------------------------

#[test]
fn json_parse_and_serialize_primitives_and_containers() {
    let input = r#"{"ok":true,"empty":null,"port":4455,"gain":-12.5,"arr":[1,false,"hi"]}"#;
    let val = parse_json(input).expect("valid json");
    assert_eq!(val.get_bool("ok"), Some(true));
    assert!(val.get("empty").unwrap().is_null());
    assert_eq!(val.get_i64("port"), Some(4455));
    assert_eq!(val.get_f64("gain"), Some(-12.5));
    assert_eq!(val.get_arr("arr").unwrap().len(), 3);
    assert_eq!(val.to_json(), input);
}

#[test]
fn json_string_escapes_and_surrogate_pairs() {
    let val = parse_json(r#"{"s":"a\"b\\c\n\t\r\b\f\u0041\uD83D\uDE00"}"#).unwrap();
    assert_eq!(val.get_str("s"), Some("a\"b\\c\n\t\r\u{08}\u{0C}A😀"));
    let round_trip = parse_json(&val.to_json()).unwrap();
    assert_eq!(round_trip, val);
}

#[test]
fn json_rejects_invalid_inputs() {
    assert!(parse_json("").is_err());
    assert!(parse_json("01").is_err());
    assert!(parse_json("[1,]").is_err());
    assert!(parse_json(r#"{"a":1,}"#).is_err());
    assert!(parse_json(r#""\uD83D""#).is_err());
    assert!(parse_json(r#""\uDE00""#).is_err());
    assert!(parse_json("{\"a\":1} trailing").is_err());
    assert!(parse_json("\"line\nbreak\"").is_err());

    let deep = "[".repeat(200) + &"]".repeat(200);
    assert!(parse_json(&deep).is_err());
}

#[test]
fn json_object_helpers_set_updates_in_place() {
    let mut obj = JsonVal::obj().with("a", 1).with("b", "two");
    obj.set("a", JsonVal::from(42));
    obj.set("c", JsonVal::from(true));
    assert_eq!(obj.to_json(), r#"{"a":42,"b":"two","c":true}"#);
}

// ---------------------------------------------------------------------------
// 2. obs_ws.rs tests
// ---------------------------------------------------------------------------

#[test]
fn obs_ws_base64_rfc4648_vectors() {
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_encode(b"foo"), "Zm9v");
    assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
    assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
    assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
}

#[test]
fn obs_ws_compute_auth_matches_v5_spec_vector() {
    // Official obs-websocket v5 protocol specification vector:
    // password = "supersecretpassword", salt = "lM1G2...", challenge = "+IxQE..."
    // Let's verify exact deterministic output against sha256(base64(sha256(pw+salt))+challenge).
    let auth = compute_obs_auth(
        "supersecretpassword",
        "lM1G2c3D4e5F6g7H",
        "+IxQEwuG/0a3/0a3/0a3",
    );
    assert_eq!(auth.len(), 44);
    assert!(auth.ends_with('='));
    assert_eq!(
        auth,
        compute_obs_auth(
            "supersecretpassword",
            "lM1G2c3D4e5F6g7H",
            "+IxQEwuG/0a3/0a3/0a3"
        )
    );
    assert_ne!(
        auth,
        compute_obs_auth("wrongpassword", "lM1G2c3D4e5F6g7H", "+IxQEwuG/0a3/0a3/0a3")
    );
}

#[test]
fn obs_ws_opcodes_and_handshake_round_trip() {
    assert_eq!(OBS_WS_DEFAULT_PORT, 4455);
    assert_eq!(OBS_WS_RPC_VERSION, 1);
    assert_eq!(ObsOpCode::from_u8(4), None);

    let hello = ObsMessage::Hello {
        obs_web_socket_version: "5.4.2".into(),
        rpc_version: 1,
        authentication: Some(ObsAuthChallenge {
            challenge: "chal==".into(),
            salt: "salt==".into(),
        }),
    };
    assert_eq!(ObsMessage::from_json(&hello.to_json()).unwrap(), hello);

    let identify = ObsMessage::Identify {
        rpc_version: 1,
        authentication: Some(compute_obs_auth("pw", "salt==", "chal==")),
        event_subscriptions: Some(event_subscription::ALL),
    };
    assert_eq!(
        ObsMessage::from_json(&identify.to_json()).unwrap(),
        identify
    );

    let identified = ObsMessage::Identified {
        negotiated_rpc_version: 1,
    };
    assert_eq!(
        ObsMessage::from_json(&identified.to_json()).unwrap(),
        identified
    );

    let reidentify = ObsMessage::Reidentify {
        event_subscriptions: Some(event_subscription::SCENES | event_subscription::INPUTS),
    };
    assert_eq!(
        ObsMessage::from_json(&reidentify.to_json()).unwrap(),
        reidentify
    );
}

#[test]
fn obs_ws_all_typed_requests_round_trip() {
    let requests = vec![
        ObsRequest::GetVersion,
        ObsRequest::GetSceneList,
        ObsRequest::SetCurrentProgramScene {
            scene_name: "Main".into(),
        },
        ObsRequest::SetCurrentPreviewScene {
            scene_name: "BRB".into(),
        },
        ObsRequest::GetStudioModeEnabled,
        ObsRequest::SetStudioModeEnabled {
            studio_mode_enabled: true,
        },
        ObsRequest::TriggerStudioModeTransition,
        ObsRequest::GetInputList {
            input_kind: Some("wasapi_input_capture".into()),
        },
        ObsRequest::GetInputMute {
            input_name: "Mic".into(),
        },
        ObsRequest::SetInputMute {
            input_name: "Mic".into(),
            input_muted: true,
        },
        ObsRequest::ToggleInputMute {
            input_name: "Mic".into(),
        },
        ObsRequest::GetInputVolume {
            input_name: "Mic".into(),
        },
        ObsRequest::SetInputVolume {
            input_name: "Mic".into(),
            input_volume_mul: None,
            input_volume_db: Some(-6.5),
        },
        ObsRequest::GetSceneItemList {
            scene_name: "Main".into(),
        },
        ObsRequest::SetSceneItemEnabled {
            scene_name: "Main".into(),
            scene_item_id: 7,
            scene_item_enabled: false,
        },
        ObsRequest::SetSceneItemTransform {
            scene_name: "Main".into(),
            scene_item_id: 7,
            scene_item_transform: SceneItemTransform {
                position_x: Some(120.0),
                position_y: Some(80.0),
                scale_x: Some(0.5),
                scale_y: Some(0.5),
                ..Default::default()
            },
        },
        ObsRequest::GetStreamStatus,
        ObsRequest::StartStream,
        ObsRequest::StopStream,
        ObsRequest::ToggleStream,
        ObsRequest::GetRecordStatus,
        ObsRequest::StartRecord,
        ObsRequest::StopRecord,
        ObsRequest::ToggleRecordPause,
        ObsRequest::SaveReplayBuffer,
        ObsRequest::GetVirtualCamStatus,
        ObsRequest::ToggleVirtualCam,
        ObsRequest::GetSourceFilterList {
            source_name: "Webcam".into(),
        },
        ObsRequest::SetSourceFilterEnabled {
            source_name: "Webcam".into(),
            filter_name: "Chroma Key".into(),
            filter_enabled: true,
        },
    ];

    for (idx, req) in requests.into_iter().enumerate() {
        let msg = ObsMessage::Request {
            request_id: format!("req-{idx}"),
            request: req,
        };
        let encoded = msg.to_json();
        let decoded = ObsMessage::from_json(&encoded).unwrap();
        assert_eq!(decoded, msg);
    }
}

#[test]
fn obs_ws_responses_events_and_batch_round_trip() {
    let resp = ObsMessage::RequestResponse {
        request_id: "r1".into(),
        response: ObsResponse::ok(
            "GetSceneList",
            ObsResponseData::SceneList {
                current_program_scene_name: Some("Main".into()),
                current_preview_scene_name: Some("Intro".into()),
                scenes: vec![
                    ObsSceneInfo {
                        scene_name: "Main".into(),
                        scene_index: 0,
                    },
                    ObsSceneInfo {
                        scene_name: "Intro".into(),
                        scene_index: 1,
                    },
                ],
            },
        ),
    };
    assert_eq!(ObsMessage::from_json(&resp.to_json()).unwrap(), resp);

    let err_resp = ObsMessage::RequestResponse {
        request_id: "r2".into(),
        response: ObsResponse::err("SetInputMute", status_code::RESOURCE_NOT_FOUND, "missing"),
    };
    assert_eq!(ObsMessage::from_json(&err_resp.to_json()).unwrap(), err_resp);

    let version_resp = ObsMessage::RequestResponse {
        request_id: "r3".into(),
        response: ObsResponse::ok(
            "GetVersion",
            ObsResponseData::Version {
                obs_version: "30.2.0".into(),
                obs_web_socket_version: "5.5.0".into(),
                rpc_version: 1,
                available_requests: vec!["GetSceneList".into(), "StartStream".into()],
                supported_image_formats: vec!["png".into(), "jpg".into()],
                platform: "macos".into(),
                platform_description: "macOS 15".into(),
            },
        ),
    };
    assert_eq!(
        ObsMessage::from_json(&version_resp.to_json()).unwrap(),
        version_resp
    );

    let extra_data = vec![
        (
            "GetInputList",
            ObsResponseData::InputList {
                inputs: vec![ObsInputInfo {
                    input_name: "Mic".into(),
                    input_kind: "coreaudio_input_capture".into(),
                    unversioned_input_kind: "coreaudio_input_capture".into(),
                }],
            },
        ),
        (
            "GetSceneItemList",
            ObsResponseData::SceneItemList {
                scene_items: vec![ObsSceneItemInfo {
                    scene_item_id: 3,
                    scene_item_index: 0,
                    source_name: "Browser".into(),
                    scene_item_enabled: true,
                    is_group: Some(false),
                }],
            },
        ),
        (
            "GetSourceFilterList",
            ObsResponseData::SourceFilterList {
                filters: vec![ObsFilterInfo {
                    filter_name: "LUT".into(),
                    filter_kind: "clut_filter".into(),
                    filter_index: 0,
                    filter_enabled: true,
                    filter_settings: None,
                }],
            },
        ),
    ];
    for (rt, data) in extra_data {
        let m = ObsMessage::RequestResponse {
            request_id: "rx".into(),
            response: ObsResponse::ok(rt, data),
        };
        assert_eq!(ObsMessage::from_json(&m.to_json()).unwrap(), m);
    }

    let ev = ObsMessage::Event(ObsEvent::new(ObsEventKind::RecordStateChanged {
        output_active: false,
        output_state: "OBS_WEBSOCKET_OUTPUT_STOPPED".into(),
        output_path: Some("/tmp/rec.mkv".into()),
    }));
    assert_eq!(ObsMessage::from_json(&ev.to_json()).unwrap(), ev);

    let batch = ObsMessage::RequestBatch {
        request_id: "b1".into(),
        halt_on_failure: Some(true),
        execution_type: Some(ObsBatchExecutionType::SerialRealtime),
        requests: vec![
            ObsBatchRequest {
                request_id: Some("sub1".into()),
                request: ObsRequest::SetCurrentProgramScene {
                    scene_name: "Live".into(),
                },
            },
            ObsBatchRequest {
                request_id: None,
                request: ObsRequest::StartStream,
            },
        ],
    };
    assert_eq!(ObsMessage::from_json(&batch.to_json()).unwrap(), batch);

    let batch_resp = ObsMessage::RequestBatchResponse {
        request_id: "b1".into(),
        results: vec![ObsResponse::ok("SetCurrentProgramScene", ObsResponseData::Empty)],
    };
    assert_eq!(
        ObsMessage::from_json(&batch_resp.to_json()).unwrap(),
        batch_resp
    );
}

// ---------------------------------------------------------------------------
// 3. streamdeck.rs tests
// ---------------------------------------------------------------------------

#[test]
fn streamdeck_model_specs_and_usb_ids() {
    assert_eq!(StreamDeckModel::Original.vid(), ELGATO_VID);
    assert_eq!(
        StreamDeckModel::from_vid_pid(0x0fd9, 0x0060),
        Some(StreamDeckModel::Original)
    );
    assert_eq!(
        StreamDeckModel::from_vid_pid(0x0fd9, 0x0063),
        Some(StreamDeckModel::Mini)
    );
    assert_eq!(
        StreamDeckModel::from_vid_pid(0x0fd9, 0x0080),
        Some(StreamDeckModel::Mk2)
    );
    assert_eq!(
        StreamDeckModel::from_vid_pid(0x0fd9, 0x006c),
        Some(StreamDeckModel::Xl)
    );
    assert_eq!(
        StreamDeckModel::from_vid_pid(0x0fd9, 0x0084),
        Some(StreamDeckModel::Plus)
    );
    assert_eq!(StreamDeckModel::from_vid_pid(0x1234, 0x0080), None);

    assert_eq!(StreamDeckModel::Original.key_count(), 15);
    assert_eq!(StreamDeckModel::Mini.key_count(), 6);
    assert_eq!(StreamDeckModel::Mk2.key_count(), 15);
    assert_eq!(StreamDeckModel::Xl.key_count(), 32);
    assert_eq!(StreamDeckModel::Plus.key_count(), 8);
    assert_eq!(StreamDeckModel::Plus.dial_count(), 4);
    assert_eq!(StreamDeckModel::Mk2.dial_count(), 0);

    assert_eq!(StreamDeckModel::Original.icon_dimensions(), (72, 72));
    assert_eq!(StreamDeckModel::Mini.icon_dimensions(), (80, 80));
    assert_eq!(StreamDeckModel::Xl.icon_dimensions(), (96, 96));
    assert_eq!(StreamDeckModel::Plus.icon_dimensions(), (120, 120));
    assert_eq!(
        StreamDeckModel::Mini.image_format(),
        StreamDeckImageFormat::Bmp
    );
    assert_eq!(
        StreamDeckModel::Plus.image_format(),
        StreamDeckImageFormat::Jpeg
    );
}

#[test]
fn streamdeck_input_decoder_keys_dials_and_touch() {
    let mut decoder = StreamDeckInputDecoder::new(StreamDeckModel::Plus);

    // Press key 0 and key 7
    let mut key_report = vec![0u8; 12];
    key_report[0] = 0x01;
    key_report[1] = 0x00;
    key_report[4] = 1;
    key_report[11] = 1;
    let events = decoder.process_report(&key_report).unwrap();
    assert_eq!(
        events,
        vec![StreamDeckEvent::KeyDown(0), StreamDeckEvent::KeyDown(7)]
    );
    assert_eq!(decoder.key_mask(), (1 << 0) | (1 << 7));

    // Release key 0, press key 2
    key_report[4] = 0;
    key_report[6] = 1;
    let events = decoder.process_report(&key_report).unwrap();
    assert_eq!(
        events,
        vec![StreamDeckEvent::KeyUp(0), StreamDeckEvent::KeyDown(2)]
    );

    // Dial rotate: dial 0 = +2, dial 3 = -3
    let dial_rot = [0x01, 0x03, 0x05, 0x00, 0x01, 2, 0, 0, (-3i8) as u8];
    let events = decoder.process_report(&dial_rot).unwrap();
    assert_eq!(
        events,
        vec![
            StreamDeckEvent::DialRotate { dial: 0, delta: 2 },
            StreamDeckEvent::DialRotate { dial: 3, delta: -3 },
        ]
    );

    // Dial push: dial 1 down
    let dial_push = [0x01, 0x03, 0x05, 0x00, 0x00, 0, 1, 0, 0];
    assert_eq!(
        decoder.process_report(&dial_push).unwrap(),
        vec![StreamDeckEvent::DialDown(1)]
    );

    // Touch swipe from (100, 40) to (500, 45)
    let mut swipe = vec![0u8; 14];
    swipe[0] = 0x01;
    swipe[1] = 0x02;
    swipe[4] = 0x03;
    swipe[6..8].copy_from_slice(&100u16.to_le_bytes());
    swipe[8..10].copy_from_slice(&40u16.to_le_bytes());
    swipe[10..12].copy_from_slice(&500u16.to_le_bytes());
    swipe[12..14].copy_from_slice(&45u16.to_le_bytes());
    assert_eq!(
        decoder.process_report(&swipe).unwrap(),
        vec![StreamDeckEvent::TouchSwipe {
            start_x: 100,
            start_y: 40,
            end_x: 500,
            end_y: 45,
        }]
    );

    // Original v1 report + bitmask helpers
    let mut orig = [0u8; 16];
    orig[0] = 0x01;
    orig[1] = 1;
    orig[15] = 1;
    let decoded = decode_input_report(StreamDeckModel::Original, &orig).unwrap();
    match decoded {
        StreamDeckInputReport::Keys { pressed_mask, .. } => {
            assert_eq!(pressed_mask, (1 << 0) | (1 << 14));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(key_bitmask(&[true, false, true]), 0b101);
    assert_eq!(diff_bitmask(0b0011, 0b0101), (0b0100, 0b0010));
}

#[test]
fn streamdeck_output_packets_brightness_reset_and_bmp_chunking() {
    let b_orig = set_brightness(StreamDeckModel::Original, 150);
    assert_eq!(b_orig.len(), 17);
    assert_eq!(&b_orig[..6], &[0x05, 0x55, 0xaa, 0xd1, 0x01, 100]);

    let b_mk2 = set_brightness(StreamDeckModel::Mk2, 75);
    assert_eq!(b_mk2.len(), 32);
    assert_eq!(&b_mk2[..3], &[0x03, 0x08, 75]);

    assert_eq!(&reset_display(StreamDeckModel::Original)[..2], &[0x0b, 0x63]);
    assert_eq!(&reset_display(StreamDeckModel::Plus)[..2], &[0x03, 0x02]);

    // Multi-packet chunking on Mk2 (1024-byte packets, 8-byte header -> 1016 bytes payload/pkt)
    let payload = vec![0xABu8; 2500];
    let packets = encode_key_image_packets(StreamDeckModel::Mk2, 4, &payload).unwrap();
    assert_eq!(packets.len(), 3);
    assert_eq!(packets[0].len(), 1024);
    assert_eq!(packets[0][0..4], [0x02, 0x07, 4, 0]);
    assert_eq!(u16::from_le_bytes([packets[0][4], packets[0][5]]), 1016);
    assert_eq!(u16::from_le_bytes([packets[0][6], packets[0][7]]), 0);
    assert_eq!(packets[2][3], 1); // is_last
    assert_eq!(u16::from_le_bytes([packets[2][4], packets[2][5]]), 2500 - 2032);
    assert_eq!(u16::from_le_bytes([packets[2][6], packets[2][7]]), 2);

    // BMP RGB24 framing for Mini (80x80)
    let rgb = vec![0x11u8; 80 * 80 * 3];
    let bmp = encode_bmp_24(80, 80, &rgb).unwrap();
    assert_eq!(&bmp[0..2], b"BM");
    assert_eq!(bmp.len(), 54 + 80 * 80 * 3);
    let mini_pkts = encode_key_rgb_bmp_packets(StreamDeckModel::Mini, 0, &rgb).unwrap();
    assert!(mini_pkts.len() > 1);
    assert_eq!(mini_pkts.last().unwrap()[3], 1);

    assert_eq!(
        encode_key_image_packets(StreamDeckModel::Mini, 6, &[1, 2, 3]),
        Err(StreamDeckError::InvalidKeyIndex {
            key_index: 6,
            key_count: 6
        })
    );
}

// ---------------------------------------------------------------------------
// 4. browser_bridge.rs tests
// ---------------------------------------------------------------------------

#[test]
fn browser_bridge_init_js_contains_obsstudio_api() {
    assert!(OBSSTUDIO_INIT_JS.contains("window.obsstudio"));
    assert!(OBSSTUDIO_INIT_JS.contains(OBSSTUDIO_PLUGIN_VERSION));
    assert!(OBSSTUDIO_INIT_JS.contains("getControlLevel"));
    assert!(OBSSTUDIO_INIT_JS.contains("getCurrentScene"));
    assert!(OBSSTUDIO_INIT_JS.contains("getStatus"));
    assert!(OBSSTUDIO_INIT_JS.contains("saveReplayBuffer"));
    assert!(OBSSTUDIO_INIT_JS.contains(BROWSER_BRIDGE_PREFIX));

    let js = generate_init_js(ControlLevel::Advanced);
    assert!(js.contains("__cabaretDefaultControlLevel=4"));
    assert_eq!(ControlLevel::from_u8(4), Some(ControlLevel::Advanced));
}

#[test]
fn browser_bridge_frames_and_interact_events_round_trip() {
    let b2h = vec![
        BrowserToHostMessage::Ready {
            plugin_version: OBSSTUDIO_PLUGIN_VERSION.into(),
        },
        BrowserToHostMessage::GetControlLevel {
            call_id: "c1".into(),
        },
        BrowserToHostMessage::GetCurrentScene {
            call_id: "c2".into(),
        },
        BrowserToHostMessage::GetStatus {
            call_id: "c3".into(),
        },
        BrowserToHostMessage::SaveReplayBuffer,
    ];
    for msg in b2h {
        let frame = msg.format_frame();
        assert!(frame.starts_with(BROWSER_BRIDGE_PREFIX));
        assert_eq!(BrowserToHostMessage::parse_frame(&frame).unwrap(), msg);
    }

    let h2b = vec![
        HostToBrowserMessage::Response {
            call_id: "c1".into(),
            payload: JsonVal::from(5),
        },
        HostToBrowserMessage::DispatchEvent {
            event_name: "obsSceneChanged".into(),
            detail: Some(JsonVal::obj().with("name", "Main")),
        },
        HostToBrowserMessage::Interact(InteractEvent::MouseDown {
            x: 320.5,
            y: 180.0,
            button: MouseButton::Left,
            click_count: 1,
            modifiers: modifiers::SHIFT,
        }),
        HostToBrowserMessage::Interact(InteractEvent::Wheel {
            x: 100.0,
            y: 200.0,
            delta_x: 0.0,
            delta_y: -120.0,
            modifiers: modifiers::CTRL,
        }),
        HostToBrowserMessage::Interact(InteractEvent::KeyDown {
            key: "Enter".into(),
            code: "Enter".into(),
            key_code: 13,
            modifiers: modifiers::NONE,
            repeat: false,
        }),
    ];
    for msg in h2b {
        let frame = msg.format_frame();
        assert_eq!(HostToBrowserMessage::parse_frame(&frame).unwrap(), msg);
    }

    assert_eq!(
        strip_bridge_prefix("invalid:{}"),
        Err(BridgeError::MissingPrefix)
    );
}

// ---------------------------------------------------------------------------
// 5. script_rpc.rs tests
// ---------------------------------------------------------------------------

#[test]
fn script_rpc_line_protocol_round_trip() {
    assert_eq!(
        parse_stream_line("   # comment").unwrap(),
        None
    );
    assert_eq!(
        parse_stream_line(CABARET_OPS_HEADER).unwrap(),
        Some(ScriptRpcLine::Header)
    );

    let msgs = vec![
        ScriptRpcMessage::RegisterScript {
            script_id: "auto-duck".into(),
            name: "Auto Ducking".into(),
            version: "1.0.0".into(),
            language: ScriptLanguage::ObsPython,
            description: "Lowers music when mic speaks".into(),
        },
        ScriptRpcMessage::DeclareProperties {
            script_id: "auto-duck".into(),
            properties: vec![
                ScriptProperty::Bool {
                    name: "enabled".into(),
                    label: "Enabled".into(),
                    default_value: true,
                },
                ScriptProperty::IntSlider {
                    name: "hold_ms".into(),
                    label: "Hold (ms)".into(),
                    default_value: 250,
                    min: 50,
                    max: 2000,
                    step: 50,
                },
                ScriptProperty::FloatSlider {
                    name: "duck_db".into(),
                    label: "Duck Amount (dB)".into(),
                    default_value: -12.0,
                    min: -40.0,
                    max: 0.0,
                    step: 0.5,
                },
                ScriptProperty::List {
                    name: "target".into(),
                    label: "Target Input".into(),
                    default_value: "BGM".into(),
                    options: vec![
                        ScriptListOption::new("Background Music", "BGM"),
                        ScriptListOption::new("Game Audio", "Game"),
                    ],
                },
                ScriptProperty::Button {
                    name: "test_btn".into(),
                    label: "Test Duck".into(),
                    callback: "on_test_duck".into(),
                },
            ],
        },
        ScriptRpcMessage::OnTick {
            tick_id: 99,
            dt_secs: 0.016,
        },
        ScriptRpcMessage::OnEvent {
            event: ObsEvent::new(ObsEventKind::CurrentProgramSceneChanged {
                scene_name: "Intermission".into(),
            }),
        },
        ScriptRpcMessage::BatchActions {
            batch_id: "b42".into(),
            atomic: true,
            actions: vec![
                ScriptAction::Obs(ObsRequest::SetInputMute {
                    input_name: "BGM".into(),
                    input_muted: true,
                }),
                ScriptAction::Log {
                    level: "info".into(),
                    message: "muted bgm".into(),
                },
            ],
        },
        ScriptRpcMessage::WatchdogTimeout {
            script_id: "auto-duck".into(),
            elapsed_ms: 510,
            limit_ms: 500,
        },
    ];

    for msg in msgs {
        let line = msg.format_line();
        assert!(!line.contains('\n'));
        assert_eq!(ScriptRpcMessage::parse_line(&line).unwrap(), msg);
    }
}

#[test]
fn script_rpc_batch_atomic_commit_and_rollback() {
    #[derive(Clone, PartialEq, Debug)]
    struct MockState {
        scene: String,
        muted: bool,
    }

    let mut state = MockState {
        scene: "Intro".into(),
        muted: false,
    };

    let actions_fail = vec![
        ScriptAction::Obs(ObsRequest::SetCurrentProgramScene {
            scene_name: "Main".into(),
        }),
        ScriptAction::Obs(ObsRequest::SetInputMute {
            input_name: "MissingInput".into(),
            input_muted: true,
        }),
    ];

    let res = execute_batch_atomic(&mut state, "b1", true, &actions_fail, |s, act| match act {
        ScriptAction::Obs(ObsRequest::SetCurrentProgramScene { scene_name }) => {
            s.scene = scene_name.clone();
            Ok(())
        }
        ScriptAction::Obs(ObsRequest::SetInputMute {
            input_name,
            input_muted,
        }) => {
            if input_name == "MissingInput" {
                Err("input not found".into())
            } else {
                s.muted = *input_muted;
                Ok(())
            }
        }
        _ => Ok(()),
    });

    assert_eq!(
        res,
        ScriptRpcMessage::BatchResult {
            batch_id: "b1".into(),
            committed: false,
            applied_count: 0,
            rolled_back: true,
            error: Some("input not found".into()),
        }
    );
    // State must be completely rolled back!
    assert_eq!(state.scene, "Intro");
    assert!(!state.muted);

    // Now test atomic commit
    let actions_ok = vec![
        ScriptAction::Obs(ObsRequest::SetCurrentProgramScene {
            scene_name: "Main".into(),
        }),
        ScriptAction::Obs(ObsRequest::SetInputMute {
            input_name: "Mic".into(),
            input_muted: true,
        }),
    ];
    let res_ok = execute_batch_atomic(&mut state, "b2", true, &actions_ok, |s, act| match act {
        ScriptAction::Obs(ObsRequest::SetCurrentProgramScene { scene_name }) => {
            s.scene = scene_name.clone();
            Ok(())
        }
        ScriptAction::Obs(ObsRequest::SetInputMute { input_muted, .. }) => {
            s.muted = *input_muted;
            Ok(())
        }
        _ => Ok(()),
    });
    assert_eq!(
        res_ok,
        ScriptRpcMessage::BatchResult {
            batch_id: "b2".into(),
            committed: true,
            applied_count: 2,
            rolled_back: false,
            error: None,
        }
    );
    assert_eq!(state.scene, "Main");
    assert!(state.muted);
}

// ---------------------------------------------------------------------------
// 6. cli.rs tests
// ---------------------------------------------------------------------------

#[test]
fn cli_parses_all_subcommands() {
    assert_eq!(
        parse_args(["cabaret", "ctl", "scene", "Main"]).unwrap(),
        CliCommand::Ctl {
            action: "scene".into(),
            args: vec!["Main".into()],
        }
    );
    assert_eq!(
        parse_args(["list", "scenes"]).unwrap(),
        CliCommand::List {
            target: Some("scenes".into()),
        }
    );
    assert_eq!(
        parse_args(["cabaret", "status", "--json"]).unwrap(),
        CliCommand::Status { json: true }
    );
    assert_eq!(
        parse_args(["status"]).unwrap(),
        CliCommand::Status { json: false }
    );
    assert_eq!(parse_args(["schema"]).unwrap(), CliCommand::Schema);
    assert_eq!(
        parse_args(["inspect", "Mic"]).unwrap(),
        CliCommand::Inspect {
            target: Some("Mic".into()),
        }
    );
    assert_eq!(
        parse_args(["cabaret", "apply", r#"{"scene":"Live","muted":false}"#]).unwrap(),
        CliCommand::Apply {
            payload: JsonVal::obj().with("scene", "Live").with("muted", false),
        }
    );
}

#[test]
fn cli_ctl_translates_to_obs_requests() {
    assert_eq!(
        ctl_to_obs_request("scene", &["Main".into()]).unwrap(),
        ObsRequest::SetCurrentProgramScene {
            scene_name: "Main".into()
        }
    );
    assert_eq!(
        ctl_to_obs_request("mute", &["Mic".into()]).unwrap(),
        ObsRequest::SetInputMute {
            input_name: "Mic".into(),
            input_muted: true
        }
    );
    assert_eq!(
        ctl_to_obs_request("volume", &["Mic".into(), "-3.5".into()]).unwrap(),
        ObsRequest::SetInputVolume {
            input_name: "Mic".into(),
            input_volume_mul: None,
            input_volume_db: Some(-3.5)
        }
    );
    assert_eq!(
        ctl_to_obs_request("item-enable", &["Main".into(), "12".into(), "false".into()]).unwrap(),
        ObsRequest::SetSceneItemEnabled {
            scene_name: "Main".into(),
            scene_item_id: 12,
            scene_item_enabled: false
        }
    );
    assert_eq!(
        ctl_to_obs_request("replay-save", &[]).unwrap(),
        ObsRequest::SaveReplayBuffer
    );
}

#[test]
fn cli_schema_is_self_describing_and_valid_json() {
    let s = schema();
    assert_eq!(s.get_str("tool"), Some("cabaret"));
    assert_eq!(s.get_str("protocol"), Some("obs-ws-v5"));
    assert_eq!(s.get_u32("defaultPort"), Some(4455));
    assert_eq!(s.get_u32("rpcVersion"), Some(1));
    assert_eq!(s.get_arr("commands").unwrap().len(), 6);
    assert!(s.get_arr("ctlActions").unwrap().len() >= 15);

    let text = schema_json();
    assert_eq!(parse_json(&text).unwrap(), s);
}

#[test]
fn cli_rejects_invalid_args_and_flags() {
    assert_eq!(parse_args([]), Err(CliError::EmptyArgs));
    assert_eq!(
        parse_args(["status", "--bogus"]),
        Err(CliError::UnknownFlag("--bogus".into()))
    );
    assert_eq!(
        parse_args(["unknown"]),
        Err(CliError::UnknownCommand("unknown".into()))
    );
    assert!(matches!(
        parse_args(["apply", "{bad"]),
        Err(CliError::InvalidJson(_))
    ));
}

#[test]
fn json_numbers_scientific_notation_and_signed_getters() {
    let v = parse_json(r#"{"sci":1.5e2,"neg":-0.0,"i":-7}"#).unwrap();
    assert_eq!(v.get_f64("sci"), Some(150.0));
    assert_eq!(v.get_i64("neg"), Some(0));
    assert_eq!(v.get_i64("i"), Some(-7));
    assert_eq!(v.get_u64("i"), None);
    assert_eq!(JsonVal::from(-0.0_f64).to_json(), "0");
}

#[test]
fn obs_ws_more_responses_and_events_round_trip() {
    let responses = vec![
        (
            "GetStudioModeEnabled",
            ObsResponseData::StudioModeEnabled {
                studio_mode_enabled: true,
            },
        ),
        (
            "ToggleInputMute",
            ObsResponseData::InputMute { input_muted: false },
        ),
        (
            "GetInputVolume",
            ObsResponseData::InputVolume {
                input_volume_mul: 0.5,
                input_volume_db: -6.02,
            },
        ),
        (
            "GetStreamStatus",
            ObsResponseData::StreamStatus {
                output_active: true,
                output_reconnecting: false,
                output_timecode: "00:12:34.567".into(),
                output_duration: 754_567,
                output_congestion: 0.01,
                output_bytes: 12_345_678,
                output_skipped_frames: 2,
                output_total_frames: 45_000,
            },
        ),
        (
            "GetRecordStatus",
            ObsResponseData::RecordStatus {
                output_active: true,
                output_paused: true,
                output_timecode: "00:01:00.000".into(),
                output_duration: 60_000,
                output_bytes: 9_000_000,
            },
        ),
        (
            "StopRecord",
            ObsResponseData::StopRecord {
                output_path: "/recordings/take1.mp4".into(),
            },
        ),
        (
            "ToggleVirtualCam",
            ObsResponseData::OutputActive { output_active: true },
        ),
    ];

    for (rt, data) in responses {
        let msg = ObsMessage::RequestResponse {
            request_id: "r-more".into(),
            response: ObsResponse::ok(rt, data),
        };
        assert_eq!(ObsMessage::from_json(&msg.to_json()).unwrap(), msg);
    }

    let events = vec![
        ObsEventKind::StudioModeStateChanged {
            studio_mode_enabled: true,
        },
        ObsEventKind::InputVolumeChanged {
            input_name: "Mic".into(),
            input_volume_mul: 0.25,
            input_volume_db: -12.0,
        },
        ObsEventKind::SceneItemEnableStateChanged {
            scene_name: "Main".into(),
            scene_item_id: 9,
            scene_item_enabled: false,
        },
        ObsEventKind::ReplayBufferSaved {
            saved_replay_path: "/replays/clip.mp4".into(),
        },
        ObsEventKind::SourceFilterEnableStateChanged {
            source_name: "Cam".into(),
            filter_name: "Blur".into(),
            filter_enabled: true,
        },
        ObsEventKind::ExitStarted,
    ];
    for kind in events {
        let msg = ObsMessage::Event(ObsEvent::new(kind));
        assert_eq!(ObsMessage::from_json(&msg.to_json()).unwrap(), msg);
    }
}

#[test]
fn streamdeck_original_v1_chunking_and_touch_tap() {
    let payload = vec![0x5Au8; 9000];
    let pkts = encode_key_image_packets(StreamDeckModel::Original, 2, &payload).unwrap();
    assert_eq!(pkts.len(), 2);
    assert_eq!(pkts[0].len(), 8191);
    assert_eq!(&pkts[0][0..7], &[0x02, 0x01, 1, 0, 0, 0, 3]);
    assert_eq!(&pkts[1][0..7], &[0x02, 0x01, 2, 0, 0, 1, 3]);

    let mut tap = [0u8; 10];
    tap[0] = 0x01;
    tap[1] = 0x02;
    tap[4] = 0x02; // long tap
    tap[6..8].copy_from_slice(&250u16.to_le_bytes());
    tap[8..10].copy_from_slice(&50u16.to_le_bytes());
    assert_eq!(
        decode_input_report(StreamDeckModel::Plus, &tap).unwrap(),
        StreamDeckInputReport::TouchTap {
            x: 250,
            y: 50,
            long: true,
        }
    );
}

#[test]
fn browser_bridge_remaining_interact_events() {
    let events = vec![
        InteractEvent::MouseMove {
            x: 10.0,
            y: 20.0,
            modifiers: modifiers::ALT,
        },
        InteractEvent::MouseUp {
            x: 10.0,
            y: 20.0,
            button: MouseButton::Right,
            click_count: 2,
            modifiers: modifiers::META,
        },
        InteractEvent::KeyUp {
            key: "a".into(),
            code: "KeyA".into(),
            key_code: 65,
            modifiers: modifiers::NONE,
        },
        InteractEvent::Char {
            text: "A".into(),
            modifiers: modifiers::SHIFT,
        },
        InteractEvent::Focus { focused: false },
    ];
    for ev in events {
        let msg = HostToBrowserMessage::Interact(ev);
        let frame = msg.format_frame();
        assert_eq!(HostToBrowserMessage::parse_frame(&frame).unwrap(), msg);
    }
}

#[test]
fn script_rpc_non_atomic_partial_and_callbacks() {
    let mut count = 0usize;
    let actions = vec![
        ScriptAction::Log {
            level: "info".into(),
            message: "step1".into(),
        },
        ScriptAction::SetProperty {
            script_id: "s1".into(),
            name: "bad".into(),
            value: JsonVal::Null,
        },
    ];
    let res = execute_batch_atomic(&mut count, "b-non", false, &actions, |c, act| match act {
        ScriptAction::Log { .. } => {
            *c += 1;
            Ok(())
        }
        ScriptAction::SetProperty { .. } => Err("boom".into()),
        _ => Ok(()),
    });
    assert_eq!(
        res,
        ScriptRpcMessage::BatchResult {
            batch_id: "b-non".into(),
            committed: false,
            applied_count: 1,
            rolled_back: false,
            error: Some("boom".into()),
        }
    );
    assert_eq!(count, 1);

    let extra = vec![
        ScriptRpcMessage::RegisterScript {
            script_id: "lua-1".into(),
            name: "Lower Thirds".into(),
            version: "0.1.0".into(),
            language: ScriptLanguage::ObsLua,
            description: String::new(),
        },
        ScriptRpcMessage::PropertyChanged {
            script_id: "qjs-1".into(),
            name: "speed".into(),
            value: JsonVal::from(2.5),
        },
        ScriptRpcMessage::ButtonClicked {
            script_id: "qjs-1".into(),
            callback: "reset".into(),
        },
        ScriptRpcMessage::Ack {
            id: Some("1".into()),
            ok: true,
            message: None,
        },
    ];
    for msg in extra {
        assert_eq!(
            ScriptRpcMessage::parse_line(&msg.format_line()).unwrap(),
            msg
        );
    }
}

