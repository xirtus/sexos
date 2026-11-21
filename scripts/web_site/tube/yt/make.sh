#!/usr/bin/env bash
# Makes the YouTube-shaped MSE fixtures (scripts/program_web_youtube_gate.py) the way
# YouTube's DASH formats are laid out: AV1 in fragmented MP4 with a sidx (like itag
# 394-399), Opus in WebM with its Cues before the Clusters (like itag 249-251), and
# VP9 in WebM for the codec switch. Synthetic pictures and tones only.
set -euo pipefail
cd "$(dirname "$0")"
q=(-hide_banner -loglevel error -y)
# 0-4 s: AV1 320x240, a key frame each second.
ffmpeg "${q[@]}" -f lavfi -i testsrc2=size=320x240:rate=25 -t 4 -c:v libaom-av1 -cpu-used 8 -crf 50 -g 25 -keyint_min 25 \
  -pix_fmt yuv420p -an -movflags +frag_keyframe+empty_moov+default_base_moof+global_sidx -f mp4 video-av1.mp4
# 4-8 s (placed with timestampOffset): VP9 160x120.
ffmpeg "${q[@]}" -f lavfi -i testsrc=size=160x120:rate=25 -t 4 -c:v libvpx-vp9 -deadline realtime -cpu-used 8 -b:v 80k -g 25 -keyint_min 25 \
  -pix_fmt yuv420p -an -f webm -dash 1 -cluster_time_limit 1000 video-vp9.webm
# Sound: 600 Hz for 0-4 s, then 750 Hz for 4-8 s.
for hz in 600 750; do
  ffmpeg "${q[@]}" -f lavfi -i sine=frequency=$hz:sample_rate=48000 -t 4 -ac 2 -c:a libopus -b:a 48k \
    -f webm -dash 1 -cluster_time_limit 1000 -cues_to_front 1 audio-$hz.webm
done
ls -l
