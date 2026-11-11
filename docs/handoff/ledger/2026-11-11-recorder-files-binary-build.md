# Files binary module wiring

PR163 head635271f3 failed actual ISO build in multiple CI lanes with six E0433 diagnostics. The Files library declared the four recorder modules, but the separate binary crate root did not. Added identical module declarations to main.rs; no runtime assertions changed. Raw job130222295733 log and runtime build artifact retained under sexos-evidence/desktop-recorder/foundation-ci635. Host custody gate had passed on that failed-build head; it did not establish binary compilation. Fresh-source CI must pass before merge.
