## 2026-11-17 CI builds the image once (WS3, replaces #257)

- New `image` job runs the mandatory entrypoint gate (`master_runtime_gate.sh`: build, provenance, 60 s probe), packs the ISO, iso_root/ and each target/*/release's own outputs (`scripts/ci_image_pack.sh`) and uploads them with the gate's serial log.
- The 13 jobs that test the default image (boot, the new browser job, devices, media, editor, programs, reading, places, desktop, js-jit ×2, productivity, terminal-editor, documents-pictures) unpack it (`.github/actions/image`) instead of each spending 20-29 minutes rebuilding it. Only profiles, recording and the input proof, which use their own build flags, still build.
- Browser gates moved out of "Boot and runtime gates" into "Browser gates", so the two halves run side by side.
- `scripts/tests/test_workflow_build_scalars.py` now runs every `run:` scalar through a real YAML parser and `bash -n`, and checks the shared jobs unpack the image and never rebuild it.
