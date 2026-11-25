# Extended memory proof is explicit on manual CI runs

- Manual `ci.yml` runs for stacked app PRs still build the complete image and run every existing host and QEMU job, including the 1 GiB Denim heap proof. The five-hour 12 GiB stress step now runs on the daily schedule or when a maintainer selects the new `extended_memory` input on a manual run.
- This matches the ordinary pull-request path, where the 12 GiB step was already skipped. It prevents the stress step from occupying one of the repository's four active workflow slots on every stacked app PR. The long proof remains available by hand and on the daily schedule.
- `scripts/tests/test_ci_memory_schedule.py` checks the default, both memory steps, and the matching extended timeout. This PR does not change any currently queued run: those retain the workflow snapshot they started with.
