# Flirt (Voice, Video & Network Explorer)

> **Identity Note**: **Flirt** (`apps/talk`, `/bin/flirt`) is the SexOS voice, video, and network communication tool, named under the romance theme (Andreas's pick). It replaces earlier diagnostic talk tools.

**Flirt** is engineered for real-time peer-to-peer audio and video communication across local networks and the Internet. It supports encrypted WebRTC audio streams, video frame transfers, screen sharing, and interactive TCP/UDP socket diagnostic exploration.

---

## Launching Flirt

Flirt requires network authority from **Collar** to establish network sockets:
```text
run +net flirt                          # Open Flirt voice interface
run +net flirt 192.168.1.50 9000        # Connect directly to peer endpoint
```
- **From Voyeur**: Press **`Super+D`**, type `flirt`, and press **`Enter`**.

---

## Operating Modes

### 1. Peer-to-Peer Voice Calls
- Connects directly to remote SexOS and SIP/Mumble audio endpoints using low-latency Opus audio codecs.
- Automatically adjusts audio stream bitrate based on **sexnet** latency metrics.

### 2. Video Calling & Screen Sharing
- Integrates with **Flash** (webcam driver) and the **Silk** desktop compositor to stream your screen or camera feed at 60 FPS with minimal CPU overhead.

### 3. Socket Explorer & Diagnostics
For network developers and system administrators:
- Interactive TCP/UDP terminal probe.
- Send and inspect raw text or binary frames across arbitrary ports with latency timing.

---

## Keyboard Shortcuts

| Shortcut | Action | Description |
|---|---|---|
| **`M`** | **Mute Microphone** | Toggle local audio input mute. |
| **`V`** | **Toggle Video** | Turn camera feed on or off. |
| **`S`** | **Screen Share** | Share active Silk workspace with connected peer. |
| **`H`** | **Hang Up** | End active call session. |
| **`Q`** | **Quit** | Close Flirt. |
