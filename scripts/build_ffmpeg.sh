#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Builds FFmpeg's libraries for SexOS programs as static archives for
# x86_64-sex-app.json (x87 and SSE, whose registers the kernel saves per task; no AVX,
# whose state it does not), with FFmpeg's hand-written SSE2/SSSE3/SSE4 assembly.
# Formats: MP4/MOV, Matroska/WebM, Ogg, MP3, ADTS AAC, FLAC and WAV; video H.264,
# HEVC, VP8, VP9, MPEG-4 Part 2 and AV1 (dav1d, scripts/build_dav1d.sh; AVIF pictures
# come through the MP4 demuxer); audio AAC, MP3, Opus, Vorbis, FLAC and PCM; plus
# libswscale and libswresample. C calls go to crates/sex-libc.
#
# Output: target/ffmpeg/lib/*.a (libdav1d.a among them) and target/ffmpeg/include.
# Programs that link FFmpeg (apps/play, apps/web) find them through SEXOS_FFMPEG
# (default: that directory).
#
# Needs clang, nasm, llvm-ar/ranlib/nm, make, and musl's headers. FFmpeg is LGPL-2.1+
# here (no GPL parts enabled); dav1d is BSD-2-Clause; musl's headers are MIT.
# Also needs meson and pkg-config for dav1d.
#
#   MUSL_INCLUDE   musl's include directory (default: found below)
#   FFMPEG_SRC     an FFmpeg checkout (default: cloned at FFMPEG_TAG)
#   FFMPEG_RECORDING=1 additionally builds FFV1 encode/decode and Matroska muxing
#   FFMPEG_RECORDING=2 also builds Splicy's export: VP9 (libvpx, scripts/build_libvpx.sh),
#                      Opus and AAC encoders, WebM and MP4 muxers
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
FFMPEG_TAG=${FFMPEG_TAG:-n7.1.2}
FFMPEG_RECORDING=${FFMPEG_RECORDING:-0}
case "$FFMPEG_RECORDING" in 0|1|2) ;; *) echo 'FFMPEG_RECORDING must be 0, 1 or 2' >&2; exit 1;; esac
OUT=$ROOT/target/ffmpeg
WORK=$OUT/work
mkdir -p "$WORK"
rm -f "$OUT/sexos-recording-v1"
RECORDING_FLAGS=()
if [[ $FFMPEG_RECORDING != 0 ]]; then
  RECORDING_FLAGS=(--enable-encoder=ffv1 --enable-decoder=ffv1 --enable-muxer=matroska)
fi
if [[ $FFMPEG_RECORDING == 2 ]]; then
  RECORDING_FLAGS+=(--enable-libvpx --enable-encoder=libvpx_vp9,opus,aac --enable-muxer=webm,mp4)
fi

if [[ -z "${MUSL_INCLUDE:-}" ]]; then
  for d in /usr/lib/musl/include /usr/include/x86_64-linux-musl /usr/local/musl/include; do
    [[ -f $d/stdio.h ]] && MUSL_INCLUDE=$d && break
  done
  if [[ -z "${MUSL_INCLUDE:-}" ]] && command -v nix >/dev/null; then
    MUSL_INCLUDE="$(nix --extra-experimental-features 'nix-command flakes' build nixpkgs#musl.dev --no-link --print-out-paths)/include"
  fi
fi
[[ -f "${MUSL_INCLUDE:-}/stdio.h" ]] || { echo "musl headers not found; set MUSL_INCLUDE" >&2; exit 1; }

if [[ -z "${FFMPEG_SRC:-}" ]]; then
  FFMPEG_SRC=$WORK/ffmpeg-$FFMPEG_TAG
  [[ -d $FFMPEG_SRC ]] || git clone -q --depth=1 --branch "$FFMPEG_TAG" https://github.com/FFmpeg/FFmpeg.git "$FFMPEG_SRC"
fi

# dav1d first: FFmpeg's configure finds it through its pkg-config file. Its file asks
# for -ldl, which is Linux's; dav1d is built without dlsym, so drop it.
MUSL_INCLUDE=$MUSL_INCLUDE bash "$ROOT/scripts/build_dav1d.sh"
DAV1D=$ROOT/target/dav1d
sed -i 's/ -pthread -ldl$//' "$DAV1D/lib/pkgconfig/dav1d.pc"
PKG_LIBDIR=$DAV1D/lib/pkgconfig
if [[ $FFMPEG_RECORDING == 2 ]]; then
  MUSL_INCLUDE=$MUSL_INCLUDE bash "$ROOT/scripts/build_libvpx.sh"
  PKG_LIBDIR=$PKG_LIBDIR:$ROOT/target/libvpx/lib/pkgconfig
fi

# musl's headers, copied so the build never depends on where they came from.
INC=$OUT/include-c
rm -rf "$INC" && mkdir -p "$INC"
cp -rL "$MUSL_INCLUDE"/. "$INC"/ && chmod -R u+w "$INC"

RES=$(clang -print-resource-dir)/include
CFLAGS="--target=x86_64-unknown-none-elf -nostdinc -isystem $RES -isystem $INC \
 -msse2 -mno-avx -mno-red-zone \
 -fPIC -ffunction-sections -fdata-sections -fno-stack-protector -D_GNU_SOURCE"

BUILD=$WORK/build
rm -rf "$BUILD" && mkdir -p "$BUILD" && cd "$BUILD"
PKG_CONFIG_LIBDIR=$PKG_LIBDIR PKG_CONFIG_PATH= "$FFMPEG_SRC/configure" --prefix="$OUT" \
  --enable-cross-compile --target-os=none --arch=x86_64 \
  --cc=clang --ar=llvm-ar --ranlib=llvm-ranlib --nm=llvm-nm --pkg-config=pkg-config --pkg-config-flags=--static \
  --extra-cflags="$CFLAGS" \
  --extra-ldflags="--target=x86_64-unknown-none-elf -nostdlib -fuse-ld=lld -Wl,--unresolved-symbols=ignore-all" \
  --x86asmexe=nasm --enable-pic \
  --disable-avx --disable-avx2 --disable-avx512 --disable-avx512icl --disable-fma3 --disable-fma4 --disable-xop \
  --disable-everything --disable-autodetect --disable-debug \
  --disable-pthreads --disable-w32threads --disable-os2threads \
  --disable-network --disable-programs --disable-doc --disable-avdevice --disable-avfilter \
  --enable-static --disable-shared --enable-swscale --enable-swresample --enable-libdav1d \
  --enable-demuxer=mov,matroska,ogg,mp3,aac,flac,wav \
  --enable-decoder=h264,hevc,vp8,vp9,mpeg4,aac,aac_fixed,mp3,mp3float,opus,vorbis,flac,pcm_s16le,pcm_f32le,libdav1d \
  --enable-parser=h264,hevc,vp8,vp9,mpeg4video,aac,mpegaudio,opus,vorbis,flac,av1 \
  --enable-bsf=vp9_superframe_split \
  "${RECORDING_FLAGS[@]}" \
  >configure.log

# Link tests always pass with no C library at configure time, so configure believes
# in every POSIX call. Turn off what sex-libc does not offer; turn on the maths it does.
OFF="ACCESS FCNTL GETAUXVAL GETHRTIME GETOPT GETRUSAGE GETTIMEOFDAY GLOB ISATTY LSTAT MKSTEMP MMAP
 MPROTECT NANOSLEEP PRCTL SCHED_GETAFFINITY SETRLIMIT STRERROR_R SYSCONF SYSCTL TEMPNAM USLEEP
 POSIX_IOCTL XMM_CLOBBERS MEMALIGN STRUCT_RUSAGE_RU_MAXRSS"
ON="ATANF ATAN2F CBRT CBRTF COPYSIGN COSF ERF EXP2 EXP2F EXPF HYPOT ISFINITE ISINF ISNAN LDEXPF
 LLRINT LLRINTF LOG2 LOG2F LOG10F LRINT LRINTF POWF RINT ROUND ROUNDF SINF TRUNC TRUNCF"
for f in $OFF; do
  sed -i "s/^#define HAVE_$f 1$/#define HAVE_$f 0/" config.h
  sed -i "s/^HAVE_$f=yes/!HAVE_$f=yes/" ffbuild/config.mak
done
for f in $ON; do
  sed -i "s/^#define HAVE_$f 0$/#define HAVE_$f 1/" config.h
  sed -i "s/^!HAVE_$f=yes/HAVE_$f=yes/" ffbuild/config.mak
done

make -j"$(nproc)" >make.log

# The program loader refuses code holding the bytes of WRPKRU, XRSTOR or XRSTORS at
# any offset (scripts/key_bytes.py). A compiler makes them by chance, as clang does in
# sha.c (`rol $0xf,%ebx; add %ebp,%edi` is c1 c3 0f 01 ef). Compile such a file
# again with other flags until its code is clean; most often a frame pointer is
# enough, since the register choice changes.
for o in $(find . -name '*.o' | sort); do
  python3 "$ROOT/scripts/key_bytes.py" "$o" >/dev/null && continue
  cmd=$(make -B -n V=1 "${o#./}" 2>/dev/null | grep -m1 -E '^(clang|nasm) ')
  [[ $cmd == clang* ]] || { echo "[ffmpeg] $o holds a refused instruction and is not C" >&2; python3 "$ROOT/scripts/key_bytes.py" "$o" >&2; exit 1; }
  fixed=0
  for extra in "-fno-omit-frame-pointer" "-O1 -fno-omit-frame-pointer" "-Os" "-O1"; do
    eval "$cmd $extra"
    if python3 "$ROOT/scripts/key_bytes.py" "$o" >/dev/null; then echo "[ffmpeg] $o: clean with $extra"; fixed=1; break; fi
  done
  [[ $fixed == 1 ]] || { echo "[ffmpeg] $o: no flags give clean code" >&2; exit 1; }
done
make -j"$(nproc)" >>make.log
make install >install.log
# Programs link dav1d from the same directory as FFmpeg.
cp "$DAV1D/lib/libdav1d.a" "$OUT/lib/"
rm -f "$OUT/lib/libvpx.a"
[[ $FFMPEG_RECORDING == 2 ]] && cp "$ROOT/target/libvpx/lib/libvpx.a" "$OUT/lib/"
printf '%s\n' "$FFMPEG_RECORDING" > "$OUT/sexos-recording-v1"
echo "[ffmpeg] $(ls "$OUT"/lib/*.a | xargs -n1 basename | tr '\n' ' ')-> $OUT"
