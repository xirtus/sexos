# Self-hosted runner on an Apple Silicon Mac

GitHub gives this public repo 20 hosted job slots at a time, and one CI run is
27 jobs (~600 job-minutes). This adds slots from a Mac.

## What runs here

Only jobs that do not boot SexOS: **Host regressions** and the three
**Host kernel-model regressions** shards (~55 job-minutes per run). Every other
job boots x86_64 QEMU and needs the hosted runners' KVM; on Apple Silicon x86
guests only run under TCG, several times slower, so they stay on GitHub.

The jobs run unchanged in an **x86_64 Ubuntu 24.04 container under Rosetta**
(Colima profile `sexos-ci`, vz VM). Same user (`runner`, passwordless sudo),
same apt packages preinstalled, same toolchain baked in.

## Routing and safety

`ci.yml` sends those jobs to `[self-hosted, sexos-mac]` only when

* the repository variable `SEXOS_MAC_RUNNER` is `on`, and
* the event is not a pull request from a fork.

Otherwise they run on `ubuntu-latest`. When the Mac is off, set the variable to
`off` (or delete it) so jobs do not wait for it:

    gh variable set SEXOS_MAC_RUNNER -R xirtus/sexos -b off

Fork pull requests can edit the workflow in their own branch, so the repo also
requires approval before any outside contributor's workflow runs (Settings >
Actions > General > "Require approval for all outside collaborators").

Runners are **ephemeral**: each container registers, takes one job, unregisters
and is deleted. Nothing from the Mac is mounted into it.

## Running it

Rosetta must be installed on the Mac (`softwareupdate --install-rosetta`). Without it
Colima silently falls back to QEMU user emulation, which is far slower and makes
gcc's linker crash (seen 2026-11-19). `run.sh` refuses to start in that case.

Needs `colima`, `docker` and an authenticated `gh` (repo admin, for runner
registration tokens).

    tools/ci/mac-runner/run.sh              # 2 slots, foreground
    SLOTS=1 tools/ci/mac-runner/run.sh

Disk: the VM disk is capped at 100 GB and grows only as used. Before each job the
supervisor needs 80 GB free on the Mac and 35 GB free in the VM; below that it
prunes and waits. After each job it deletes the container, prunes Docker and
runs `fstrim` so freed space returns to macOS. To give everything back:

    colima delete sexos-ci
