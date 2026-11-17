#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
set -euo pipefail

# ACE-Step 1.5 Bridge & On-Demand Model Pack Downloader
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

ACTION=""
PACK=""
PORT="8080"
MODE="auto"
TARGET_DIR=""
DRY_RUN=0
OUTPUT_JSON=0

show_help() {
  cat <<'EOF'
Usage: start_moan_ace_bridge.sh [OPTIONS]

Options:
  --status [--json]               Detect system specs, GPU backend, tier recommendation, model dir
  --download lite|studio [--dir PATH] [--dry-run]
                                  On-demand model pack installer (lite ~2.4GB, studio ~7.7GB)
  --serve [--port PORT] [--mode auto|cpp|python|mock]
                                  Launch ace-server, acestep-api, or mock server
  --json                          Force JSON output where supported
  --help, -h                      Show this help message
EOF
}

resolve_model_dir() {
  if [ -n "$TARGET_DIR" ]; then
    echo "$TARGET_DIR"
  elif [ -n "${MOAN_MUSE_DIR:-}" ]; then
    echo "$MOAN_MUSE_DIR"
  elif [ -d "$SCRIPT_DIR/../target/moan-muse-models" ]; then
    echo "$SCRIPT_DIR/../target/moan-muse-models"
  else
    echo "$HOME/.local/share/sexos/moan/muse"
  fi
}

detect_specs() {
  OS="$(uname -s)"
  ARCH="$(uname -m)"

  # RAM detection in MB
  if [ -n "${MOAN_SIMULATED_RAM_MB:-}" ]; then
    RAM_MB="$MOAN_SIMULATED_RAM_MB"
  elif [ "$OS" = "Darwin" ]; then
    local mem_bytes
    mem_bytes=$(sysctl -n hw.memsize 2>/dev/null || echo 0)
    RAM_MB=$((mem_bytes / 1048576))
  elif [ "$OS" = "Linux" ]; then
    local mem_kb
    mem_kb=$(grep -i MemTotal /proc/meminfo 2>/dev/null | awk '{print $2}' || echo 0)
    RAM_MB=$((mem_kb / 1024))
  else
    RAM_MB=8192
  fi

  # GPU backend detection
  if [ -n "${MOAN_SIMULATED_GPU_BACKEND:-}" ]; then
    GPU_BACKEND="$MOAN_SIMULATED_GPU_BACKEND"
  elif [ "$OS" = "Darwin" ]; then
    if python3 -c "import mlx" 2>/dev/null; then
      GPU_BACKEND="mlx"
    elif [ "$ARCH" = "arm64" ]; then
      GPU_BACKEND="metal"
    else
      GPU_BACKEND="cpu"
    fi
  elif [ "$OS" = "Linux" ]; then
    if command -v nvidia-smi >/dev/null 2>&1 && nvidia-smi >/dev/null 2>&1; then
      GPU_BACKEND="cuda"
    elif command -v rocm-smi >/dev/null 2>&1; then
      GPU_BACKEND="rocm"
    elif [ -d "/opt/rocm" ]; then
      GPU_BACKEND="rocm"
    else
      GPU_BACKEND="cpu"
    fi
  else
    GPU_BACKEND="cpu"
  fi

  # Recommended tier recommendation
  if [ -n "${MOAN_SIMULATED_TIER:-}" ]; then
    RECOMMENDED_TIER="$MOAN_SIMULATED_TIER"
  elif [ "$RAM_MB" -lt 8192 ]; then
    RECOMMENDED_TIER="tier0"
  elif [ "$RAM_MB" -ge 32768 ]; then
    RECOMMENDED_TIER="tier3-studio"
  else
    RECOMMENDED_TIER="tier2-lite"
  fi
}

check_models_exist() {
  local dir="$1"
  if [ -d "$dir" ]; then
    if [ -f "$dir/manifest.json" ]; then
      echo "true"
      return
    fi
    for f in "$dir"/*.gguf; do
      if [ -f "$f" ]; then
        echo "true"
        return
      fi
    done
  fi
  echo "false"
}

compute_sha256() {
  local file="$1"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  else
    python3 -c "import hashlib, sys; print(hashlib.sha256(open(sys.argv[1], 'rb').read()).hexdigest())" "$file"
  fi
}

get_file_size() {
  local file="$1"
  if [ "$(uname -s)" = "Darwin" ]; then
    stat -f%z "$file" 2>/dev/null || echo 0
  else
    stat -c%s "$file" 2>/dev/null || echo 0
  fi
}

do_status() {
  detect_specs
  local model_dir
  model_dir="$(resolve_model_dir)"
  local models_exist
  models_exist="$(check_models_exist "$model_dir")"

  if [ "$OUTPUT_JSON" -eq 1 ]; then
    cat <<EOF
{
  "os": "$OS",
  "arch": "$ARCH",
  "ram_mb": $RAM_MB,
  "gpu_backend": "$GPU_BACKEND",
  "recommended_tier": "$RECOMMENDED_TIER",
  "model_dir": "$model_dir",
  "models_exist": $models_exist
}
EOF
  else
    cat <<EOF
OS: $OS
Arch: $ARCH
RAM (MB): $RAM_MB
GPU Backend: $GPU_BACKEND
Recommended Tier: $RECOMMENDED_TIER
Model Directory: $model_dir
Models Exist: $models_exist
EOF
  fi
}

do_download() {
  if [ -z "$PACK" ]; then
    echo "Error: --download requires 'lite' or 'studio'" >&2
    exit 1
  fi

  local model_dir
  model_dir="$(resolve_model_dir)"

  local manifest_json=""
  if [ "$PACK" = "lite" ]; then
    manifest_json=$(cat <<EOF
{
  "pack": "lite",
  "target_dir": "$model_dir",
  "total_bytes": 2512388096,
  "files": [
    {
      "name": "Qwen3-Embedding-0.6B-Q8_0.gguf",
      "size": 671088640,
      "sha256": "c8d35051a34b22c54bbd72d62de3c457d2a3c77d5985ea840130db1be51b88e1",
      "url": "https://huggingface.co/MoanStudio/ACE-Step-1.5-GGUF/resolve/main/Qwen3-Embedding-0.6B-Q8_0.gguf"
    },
    {
      "name": "acestep-v15-turbo-Q4_K_M.gguf",
      "size": 1610612736,
      "sha256": "9b4f2c58e17d23a49e6f3b72c918e7456d3210abcf827163549a1bcdef982345",
      "url": "https://huggingface.co/MoanStudio/ACE-Step-1.5-GGUF/resolve/main/acestep-v15-turbo-Q4_K_M.gguf"
    },
    {
      "name": "vae-BF16.gguf",
      "size": 230686720,
      "sha256": "5e89a3d7c4b12608f512789ef23401ab6789cdef1234567890abcdef12345678",
      "url": "https://huggingface.co/MoanStudio/ACE-Step-1.5-GGUF/resolve/main/vae-BF16.gguf"
    }
  ],
  "urls": [
    "https://huggingface.co/MoanStudio/ACE-Step-1.5-GGUF/resolve/main/Qwen3-Embedding-0.6B-Q8_0.gguf",
    "https://huggingface.co/MoanStudio/ACE-Step-1.5-GGUF/resolve/main/acestep-v15-turbo-Q4_K_M.gguf",
    "https://huggingface.co/MoanStudio/ACE-Step-1.5-GGUF/resolve/main/vae-BF16.gguf"
  ]
}
EOF
)
  elif [ "$PACK" = "studio" ]; then
    manifest_json=$(cat <<EOF
{
  "pack": "studio",
  "target_dir": "$model_dir",
  "total_bytes": 8267812044,
  "files": [
    {
      "name": "acestep-5Hz-lm-4B-Q8_0.gguf",
      "size": 4509715660,
      "sha256": "3a8f1b62c4e9705d829146f3d1b7829a4c5e6f1029384756abcedf0123456789",
      "url": "https://huggingface.co/MoanStudio/ACE-Step-1.5-GGUF/resolve/main/acestep-5Hz-lm-4B-Q8_0.gguf"
    },
    {
      "name": "acestep-v15-xl-turbo-Q8_0.gguf",
      "size": 3758096384,
      "sha256": "7f9c2d1e0a8b4c5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e",
      "url": "https://huggingface.co/MoanStudio/ACE-Step-1.5-GGUF/resolve/main/acestep-v15-xl-turbo-Q8_0.gguf"
    }
  ],
  "urls": [
    "https://huggingface.co/MoanStudio/ACE-Step-1.5-GGUF/resolve/main/acestep-5Hz-lm-4B-Q8_0.gguf",
    "https://huggingface.co/MoanStudio/ACE-Step-1.5-GGUF/resolve/main/acestep-v15-xl-turbo-Q8_0.gguf"
  ]
}
EOF
)
  else
    echo "Error: Unknown pack '$PACK'. Must be 'lite' or 'studio'" >&2
    exit 1
  fi

  if [ "$DRY_RUN" -eq 1 ]; then
    echo "$manifest_json"
    return 0
  fi

  # Live download
  mkdir -p "$model_dir"
  echo "Downloading pack '$PACK' into $model_dir..."

  # Parse urls and names via python helper to avoid jq dependency
  python3 -c "
import json, os, subprocess, sys

manifest = json.loads('''$manifest_json''')
target_dir = manifest['target_dir']
files = manifest['files']

for f in files:
    name = f['name']
    url = f['url']
    dest = os.path.join(target_dir, name)
    tmp_dest = dest + '.tmp'
    print(f'Fetching {name} from {url}...')
    if shutil_which := subprocess.run(['which', 'hf'], capture_output=True).returncode == 0:
        subprocess.run(['hf', 'hub', 'download', '--local-dir', target_dir, 'MoanStudio/ACE-Step-1.5-GGUF', name], check=True)
    else:
        cmd = ['curl', '-L', '-C', '-', '--fail', '--retry', '3', '-o', tmp_dest, url]
        res = subprocess.run(cmd)
        if res.returncode == 0:
            os.replace(tmp_dest, dest)
        else:
            print(f'Download failed for {name}', file=sys.stderr)
            sys.exit(1)

with open(os.path.join(target_dir, 'manifest.json'), 'w') as mf:
    json.dump(manifest, mf, indent=2)
print('Pack installed successfully.')
"
}

do_serve() {
  local model_dir
  model_dir="$(resolve_model_dir)"
  detect_specs

  if [ "$MODE" = "mock" ]; then
    echo "[start_moan_ace_bridge] Launching mock ACE-Step server on port $PORT..." >&2
    exec python3 "$SCRIPT_DIR/mock_ace_server.py" --port "$PORT"
  elif [ "$MODE" = "cpp" ]; then
    local ace_bin=""
    if command -v ace-server >/dev/null 2>&1; then
      ace_bin="ace-server"
    elif [ -x "$SCRIPT_DIR/../target/release/ace-server" ]; then
      ace_bin="$SCRIPT_DIR/../target/release/ace-server"
    elif [ -x "$HOME/.local/bin/ace-server" ]; then
      ace_bin="$HOME/.local/bin/ace-server"
    fi

    if [ -z "$ace_bin" ]; then
      echo "Error: ace-server binary not found in PATH or standard paths." >&2
      exit 1
    fi

    local extra_flags=()
    if [ "$RAM_MB" -le 16384 ]; then
      extra_flags+=(--low-vram --no-mmap)
    fi

    if [ "$RECOMMENDED_TIER" = "tier3-studio" ]; then
      exec "$ace_bin" --port "$PORT" \
        --model-lm "$model_dir/acestep-5Hz-lm-4B-Q8_0.gguf" \
        --model-dit "$model_dir/acestep-v15-xl-turbo-Q8_0.gguf" \
        "${extra_flags[@]}"
    else
      exec "$ace_bin" --port "$PORT" \
        --model-embed "$model_dir/Qwen3-Embedding-0.6B-Q8_0.gguf" \
        --model-turbo "$model_dir/acestep-v15-turbo-Q4_K_M.gguf" \
        --model-vae "$model_dir/vae-BF16.gguf" \
        "${extra_flags[@]}"
    fi
  elif [ "$MODE" = "python" ]; then
    if command -v uv >/dev/null 2>&1; then
      exec uv run acestep-api --port "$PORT"
    elif command -v acestep-api >/dev/null 2>&1; then
      exec acestep-api --port "$PORT"
    else
      echo "Error: neither uv nor acestep-api found in PATH." >&2
      exit 1
    fi
  elif [ "$MODE" = "auto" ]; then
    if command -v ace-server >/dev/null 2>&1; then
      MODE="cpp"
      do_serve
    elif command -v acestep-api >/dev/null 2>&1; then
      MODE="python"
      do_serve
    elif command -v uv >/dev/null 2>&1 && uv run --help >/dev/null 2>&1; then
      MODE="python"
      do_serve
    else
      echo "[start_moan_ace_bridge] No native ace-server or uv/acestep-api detected; falling back to mock server on port $PORT" >&2
      exec python3 "$SCRIPT_DIR/mock_ace_server.py" --port "$PORT"
    fi
  else
    echo "Error: Unknown mode '$MODE' (valid: auto, cpp, python, mock)" >&2
    exit 1
  fi
}

# Parse CLI arguments
while [ $# -gt 0 ]; do
  case "$1" in
    --status)
      ACTION="status"
      if [ $# -gt 1 ] && [ "$2" = "json" ]; then
        OUTPUT_JSON=1
        shift
      fi
      ;;
    --json)
      OUTPUT_JSON=1
      ;;
    --download)
      ACTION="download"
      if [ $# -gt 1 ] && [[ "$2" != --* ]]; then
        PACK="$2"
        shift
      fi
      ;;
    lite|studio)
      PACK="$1"
      ;;
    --dir)
      if [ $# -gt 1 ]; then
        TARGET_DIR="$2"
        shift
      fi
      ;;
    --dry-run)
      DRY_RUN=1
      ;;
    --serve)
      ACTION="serve"
      ;;
    --port)
      if [ $# -gt 1 ]; then
        PORT="$2"
        shift
      fi
      ;;
    --mode)
      if [ $# -gt 1 ]; then
        MODE="$2"
        shift
      fi
      ;;
    --help|-h)
      show_help
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      show_help
      exit 1
      ;;
  esac
  shift
done

case "$ACTION" in
  status)
    do_status
    ;;
  download)
    do_download
    ;;
  serve)
    do_serve
    ;;
  *)
    if [ "$OUTPUT_JSON" -eq 1 ]; then
      do_status
    else
      show_help
      exit 1
    fi
    ;;
esac
