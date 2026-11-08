#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Builds FFmpeg's libraries for SexOS programs: libavformat (MP4/MOV), libavcodec
# (H.264), libswscale and libavutil, as static archives for x86_64-sex.json's
# soft-float, no-SSE, no-red-zone code. C calls go to crates/sex-libc.
#
# Output: target/ffmpeg/lib/*.a and target/ffmpeg/include. Programs that link FFmpeg
# (apps/play) find them through SEXOS_FFMPEG (default: that directory).
#
# Needs clang, llvm-ar/ranlib/nm, make, and musl's headers. FFmpeg is LGPL-2.1+
# here (no GPL parts enabled); musl's headers are MIT.
#
#   MUSL_INCLUDE   musl's include directory (default: found below)
#   FFMPEG_SRC     an FFmpeg checkout (default: cloned at FFMPEG_TAG)
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
FFMPEG_TAG=${FFMPEG_TAG:-n7.1.2}
OUT=$ROOT/target/ffmpeg
WORK=$OUT/work
mkdir -p "$WORK"

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

# musl's headers with `long double` as `double`: the soft-float target has no x87,
# so clang refuses a real long double.
INC=$OUT/include-c
rm -rf "$INC" && mkdir -p "$INC"
cp -rL "$MUSL_INCLUDE"/. "$INC"/ && chmod -R u+w "$INC"
grep -rl "long double" "$INC" | xargs -r sed -i 's/long double/double/g'

# The one `long double` literal FFmpeg compiles here.
sed -i 's/1000000000\.0L \* y/1000000000.0 * y/' "$FFMPEG_SRC/libavutil/avsscanf.c"

RES=$(clang -print-resource-dir)/include
CFLAGS="--target=x86_64-unknown-none-elf -nostdinc -isystem $RES -isystem $INC \
 -mno-sse -mno-sse2 -mno-mmx -mno-80387 -msoft-float -mno-red-zone \
 -U__FLT_EVAL_METHOD__ -D__FLT_EVAL_METHOD__=0 -Wno-builtin-macro-redefined \
 -fPIC -ffunction-sections -fdata-sections -fno-stack-protector -D_GNU_SOURCE"

BUILD=$WORK/build
rm -rf "$BUILD" && mkdir -p "$BUILD" && cd "$BUILD"
"$FFMPEG_SRC/configure" --prefix="$OUT" \
  --enable-cross-compile --target-os=none --arch=x86_64 \
  --cc=clang --ar=llvm-ar --ranlib=llvm-ranlib --nm=llvm-nm --pkg-config=false \
  --extra-cflags="$CFLAGS" \
  --extra-ldflags="--target=x86_64-unknown-none-elf -nostdlib -fuse-ld=lld -Wl,--unresolved-symbols=ignore-all" \
  --disable-everything --disable-autodetect --disable-asm --disable-debug --enable-small \
  --disable-runtime-cpudetect --disable-pthreads --disable-w32threads --disable-os2threads \
  --disable-network --disable-programs --disable-doc --disable-avdevice --disable-avfilter \
  --disable-swresample --enable-static --disable-shared \
  --enable-decoder=h264 --enable-parser=h264 --enable-demuxer=mov --enable-swscale \
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
make install >install.log
echo "[ffmpeg] $(ls "$OUT"/lib/*.a | xargs -n1 basename | tr '\n' ' ')-> $OUT"
