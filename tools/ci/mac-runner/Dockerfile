# x86_64 Ubuntu 24.04 image for an ephemeral self-hosted GitHub Actions runner.
# On an Apple Silicon Mac it runs under Rosetta in a Colima VM (see README.md),
# so CI jobs written for ubuntu-latest run unchanged.
FROM --platform=linux/amd64 ubuntu:24.04

ARG RUNNER_VERSION=2.338.2
ARG RUST_TOOLCHAIN=nightly-2026-07-22
ENV DEBIAN_FRONTEND=noninteractive LANG=C.UTF-8
# The runner is .NET; under Rosetta its W^X JIT mapping crashes (AccessViolation).
ENV DOTNET_EnableWriteXorExecute=0

# Base tools the hosted image has, plus everything .github/actions/setup and the
# host jobs install, so their apt steps become no-ops.
RUN rm -f /var/lib/man-db/auto-update \
 && apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends \
      ca-certificates curl git git-lfs jq sudo unzip zip xz-utils zstd file lsb-release \
      build-essential pkg-config python3 python3-pip python3-venv python3-pil \
      ripgrep clang lld llvm wabt nasm musl-dev cmake ninja-build meson acpica-tools mtools gdisk dosfstools ruby \
      xorriso qemu-system-x86 qemu-utils genisoimage udftools fdisk e2fsprogs \
      ffmpeg libavcodec-dev libavformat-dev libavutil-dev libswscale-dev \
      openssh-server openssh-sftp-server aria2 transmission-cli \
      libicu74 libkrb5-3 zlib1g liblttng-ust1t64 \
 && rm -rf /var/lib/apt/lists/*

# Same account shape as GitHub's hosted runners: user "runner", passwordless sudo.
RUN useradd -m -s /bin/bash runner \
 && echo 'runner ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/runner && chmod 0440 /etc/sudoers.d/runner

USER runner
WORKDIR /home/runner
ENV PATH=/home/runner/.cargo/bin:$PATH

RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none \
 && rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal -c rust-src -t x86_64-unknown-none \
 && rustup default "$RUST_TOOLCHAIN"

RUN mkdir actions-runner && cd actions-runner \
 && curl -sSfL "https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/actions-runner-linux-x64-${RUNNER_VERSION}.tar.gz" | tar xz

COPY --chown=runner:runner entrypoint.sh /home/runner/entrypoint.sh
ENTRYPOINT ["/home/runner/entrypoint.sh"]
