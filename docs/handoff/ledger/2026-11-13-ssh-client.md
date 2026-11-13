# SSH client (workstream K7)

- `crates/sex-ssh`: sans-I/O SSH client written for SexOS: curve25519-sha256 with strict KEX, AES-256/128-GCM, host keys ssh-ed25519 / ecdsa-sha2-nistp256 / rsa-sha2-256/512, ed25519 user keys (OpenSSH key files read and written; encrypted ones refused for now), password auth, session channel with exec, PTY + shell, window flow control, server rekey. Host tests against real OpenSSH sshd for all three host key types, 5 MB of output, and a PTY shell.
- `apps/ssh` (std program): `run +net ssh [-p PORT] [-i KEY] [--password] USER@HOST [COMMAND]`. The first run makes `id_ed25519` and prints its authorized_keys line; host keys are remembered in `known_hosts` (a changed key is refused). Keys come from RDRAND hashed with SHA-256; without RDRAND it refuses to run.
- Gate `scripts/ssh_gate.py`: on a fresh disk the program makes its key and is refused, the gate authorizes it, and the second run logs in to the host's sshd and gets `echo` output and status 0.
- Shell input is line-based (the std console gives lines); a full-screen terminal needs Term integration later.
