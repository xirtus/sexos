# Voice for SexOS

Run `voice` with its optional network grant and a MOSSlanding bridge:

```text
run +net voice http://10.0.2.2:8875
run +net voice http://10.0.2.2:8875 8
```

The second form selects voice ID 8. The bridge maps IDs to existing MOSSlanding
voice filenames through `MOSS_VOICE_MAP`, such as `{"8":"voice.wav"}`. ID 0
requests direct synthesis. A bare `run voice` can open the window, but its
network request is refused because Shell did not grant that run the network.

Type up to 1024 ASCII bytes. Ctrl+G submits one job, Ctrl+C cancels a pending
job, and F11 closes the window. Each completed take is fetched as 32 KiB byte
ranges, capped at 64 MiB, checked against the result's SHA-256, and saved by
SexFiles under the program's `/data/voice` folder. The WAV is published only
after verification; `take-JOB.partial` remains after an interrupted transfer
for inspection. `take-JOB.sxvc` holds the result frame and `take-JOB.txt` holds
the exact prompt. Existing filenames are never replaced.

This slice supports MOSSlanding synthesis with a voice already configured on
the worker. It does not import a new local reference recording or play audio
in the app. The worker is a separate host process; run
`python3 crates/sex-voice/adapters/mosslanding.py --port 8875` with MOSSlanding
available at its default `http://127.0.0.1:8765`, or add `--fixture` for a
deterministic test WAV. The default guest URL `10.0.2.2` is QEMU's host address.
