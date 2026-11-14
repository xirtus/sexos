# Qupid (Media Player)

> **Identity Note**: **Qupid** (`apps/play`, `/bin/qupid`) is the native SexOS media player, named under the romance theme. It replaces earlier prototypes and incorporates high-performance FFmpeg video/audio decoders.

**Qupid** is a media player engineered for smooth, frame-accurate video and high-fidelity sound playback. It features hardware-accelerated scanout, subtitle rendering, stream buffering, playlists, and resume-watching history.

---

## Launching Qupid

Qupid can play local files or stream media over the network:
```text
run qupid @video.mp4                  # Play local video file with custody grant
run qupid @song.flac                  # Play lossless audio track
run +net qupid https://media.site/hls # Stream online media with network authority
```

---

## Playback Controls & Shortcuts

| Shortcut | Action | Description |
|---|---|---|
| **`Space`** | **Play / Pause** | Toggle video and audio playback. |
| **`Left / Right`** | **Quick Seek** | Skip backward or forward by 5 seconds. |
| **`Shift+Left / Right`**| **Major Seek** | Jump backward or forward by 30 seconds. |
| **`Up / Down`** | **Volume** | Increase or decrease audio playback volume. |
| **`M`** | **Mute** | Toggle audio mute. |
| **`F`** | **Fullscreen** | Expand playback frame to fill physical monitor. |
| **`S`** | **Subtitles** | Cycle through embedded and external subtitle streams (SRT, ASS). |
| **`A`** | **Audio Track** | Cycle through available multi-language audio streams. |
| **`[` / `]`** | **Speed** | Decrease or increase playback speed (0.5× to 2.0×). |
| **`Backspace`** | **Restart** | Return to the start of the current media file. |
| **`Q` / `Esc`** | **Quit** | Stop playback and close window. |

---

## Key Features

### 1. Broad Codec & Format Compatibility
Powered by a native FFmpeg decoding engine, Qupid plays all standard video and audio formats:
- **Video**: H.264 (AVC), H.265 (HEVC), AV1, VP9, MPEG-4.
- **Audio**: FLAC, ALAC, MP3, AAC, Opus, Vorbis, PCM WAV.
- **Containers**: MP4, MKV, WebM, OGG, AVI, MOV.

### 2. Up-Next & Keep-Watching Cards
Qupid tracks playback progress safely within its private sandbox. When you re-open an episodic series or long movie, Qupid displays an "Up Next" summary card and automatically resumes playback at the exact second you left off.

### 3. Integrated Audio Mixer
Audio streams from Qupid connect directly to the SexOS sound server (**Bell** / `sexdisplay`), allowing smooth mixing alongside system alert chimes and voice calls in **[Flirt](flirt.md)**.
