#!/usr/bin/env bash
# Host-side supervisor for the SexOS self-hosted runner on an Apple Silicon Mac.
# Starts SLOTS ephemeral x86_64 runner containers in the Colima profile "sexos-ci";
# each container takes one job and is deleted, so nothing from a job survives it.
#
#   tools/ci/mac-runner/run.sh            # run in the foreground (Ctrl-C stops)
#   SLOTS=1 tools/ci/mac-runner/run.sh
#
# Disk safety: before every job it checks the Mac's free space and the VM's
# free space, prunes Docker leftovers, and trims the VM disk so freed blocks go
# back to macOS. Below the floor it waits instead of taking work.
set -uo pipefail

REPO="${REPO:-xirtus/sexos}"
PROFILE="${PROFILE:-sexos-ci}"
SLOTS="${SLOTS:-2}"
IMAGE="${IMAGE:-sexos-ci-runner:latest}"
HOST_FLOOR_GB="${HOST_FLOOR_GB:-80}"   # Mac free space below which no job starts
VM_FLOOR_GB="${VM_FLOOR_GB:-35}"       # VM free space a job needs (host job target/ is ~20-30 GB)
JOB_CPUS="${JOB_CPUS:-4}"
JOB_MEM="${JOB_MEM:-5g}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export DOCKER_CONTEXT="colima-$PROFILE"

log() { printf '%s [mac-runner] %s\n' "$(date '+%F %T')" "$*"; }

host_free_gb() { df -g "$HOME" | awk 'NR==2 {print $4}'; }
vm_free_gb() { colima ssh -p "$PROFILE" -- df -BG --output=avail /var/lib/docker 2>/dev/null | tail -1 | tr -dc 0-9; }

cleanup() {
  docker container prune -f >/dev/null 2>&1
  docker volume prune -af >/dev/null 2>&1
  docker image prune -f >/dev/null 2>&1
  docker builder prune -af >/dev/null 2>&1
  colima ssh -p "$PROFILE" -- sudo fstrim -a >/dev/null 2>&1 || true
}

wait_for_space() {
  while :; do
    local h v; h="$(host_free_gb)"; v="$(vm_free_gb)"
    if [ "${h:-0}" -ge "$HOST_FLOOR_GB" ] && [ "${v:-0}" -ge "$VM_FLOOR_GB" ]; then return; fi
    log "low disk (Mac ${h:-?} GB free, floor $HOST_FLOOR_GB; VM ${v:-?} GB free, floor $VM_FLOOR_GB); cleaning and waiting"
    cleanup; sleep 120
  done
}

slot() {
  local n="$1"
  while :; do
    wait_for_space
    local token
    if ! token="$(gh api -X POST "repos/$REPO/actions/runners/registration-token" -q .token)"; then
      log "slot $n: could not get a registration token; retrying in 60 s"; sleep 60; continue
    fi
    log "slot $n: waiting for a job"
    local t0="$SECONDS"
    docker run --rm --name "sexos-mac-$n" --platform linux/amd64 \
      --cpus "$JOB_CPUS" --memory "$JOB_MEM" --memory-swap "$JOB_MEM" \
      -e RUNNER_URL="https://github.com/$REPO" -e RUNNER_TOKEN="$token" \
      -e RUNNER_NAME="$(hostname -s)-sexos-mac-$n" -e RUNNER_LABELS="sexos-mac" \
      "$IMAGE"
    local rc=$?
    log "slot $n: runner exited ($rc), cleaning up"
    cleanup
    # A runner that dies at once (bad image, auth trouble) must not spin.
    if [ $((SECONDS - t0)) -lt 60 ]; then log "slot $n: exited within a minute; backing off 5 min"; sleep 300; fi
  done
}

/usr/bin/arch -x86_64 /usr/bin/true 2>/dev/null || { log "Rosetta is not installed (softwareupdate --install-rosetta); refusing to run x86 jobs under QEMU"; exit 1; }
colima status -p "$PROFILE" >/dev/null 2>&1 || colima start "$PROFILE" --vm-type vz --vz-rosetta --arch aarch64 --cpu 8 --memory 10 --disk 100 --runtime docker
colima ssh -p "$PROFILE" -- test -e /proc/sys/fs/binfmt_misc/rosetta || { log "the VM has no Rosetta binfmt; restart it: colima stop $PROFILE"; exit 1; }
docker image inspect "$IMAGE" >/dev/null 2>&1 || docker build --platform linux/amd64 -t "$IMAGE" "$HERE"

trap 'log "stopping"; docker ps -q --filter name=sexos-mac- | xargs docker stop -t 30 >/dev/null; kill 0' INT TERM
for n in $(seq 1 "$SLOTS"); do slot "$n" & sleep 5; done
wait
