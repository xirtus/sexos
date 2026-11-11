# Shell cleanup after Files restart

A live new Files epoch cannot validate old Read/Bind success. Shell now stops the session and sends only the exact original image Release (same nonce and complete original scope), allowing the restored KEEP obligation to be surrendered. Retries are bounded to one per 200ms. Only matching actual Files Release/Ok clears local custody; it always terminates Failed and never loads, rebinds, or rebirths an image across the restart. Unknown epoch, quarantine and denial remain fenced.

Added production controller fixture for old Read success racing restart, exact old-scope Release, and absence of load/rebirth/bind. CI validation pending; native KEEP/backing and restart proof still required. Display committed-indicator/capture integration a34244c3 is included as source only; its targeted eight controls passed on the owning worktree. No BOOT grants or activation added.
