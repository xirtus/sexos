# Decoded PTS tied to actual Display publications

The app emits a bounded (maximum301) receipt after each successful encoder admission: exact session/task, sequential index, copied-frame generation, original publication_ms and admitted pts_ms. This uses already private metadata, performs no shared read or allocation, and does not influence capture/codec decisions. Saved now carries the same exact session/task.

The native gate correlates each encoder receipt to a preceding actual Display publication with the same generation/time, rejects duplicate/reordered/missing identities, and checks decoded packet PTS against the original publication-time deltas. Published frames not admitted by the encoder are explicitly retained as dropped_publications. Other session/task completion cannot satisfy Saved or frame receipt matching. The trace helper is hashed into the evidence receipt.

Five targeted Python controls pass: preserved600ms gap with an explicit dropped frame; wrong timestamp/generation/index; wrong session/task; receipt-before-publication; duplicate/missing receipt. Script syntax and diff checks pass. No compiler or VM ran during the shared UX timing block. This remains a timestamp/ordering/output proof, not exact framebuffer pixel equality or isolation proof. The bounded diagnostic adds serial work; no performance claim is made from this image.
