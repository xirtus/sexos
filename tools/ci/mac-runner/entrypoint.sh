#!/usr/bin/env bash
# Registers an ephemeral runner (one job, then it unregisters itself) and runs it.
set -euo pipefail
: "${RUNNER_URL:?}" "${RUNNER_TOKEN:?}" "${RUNNER_NAME:?}"
cd /home/runner/actions-runner
./config.sh --unattended --ephemeral --replace --disableupdate \
  --url "$RUNNER_URL" --token "$RUNNER_TOKEN" --name "$RUNNER_NAME" \
  --labels "${RUNNER_LABELS:-sexos-mac}" --work /home/runner/work
unset RUNNER_TOKEN
exec ./run.sh
