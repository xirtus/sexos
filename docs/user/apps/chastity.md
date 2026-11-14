# Chastity (Password Vault)

> **Identity Note**: **Chastity** (`apps/chastity`, `crates/chastity-core`) is the native SexOS encrypted password manager and credential vault, named under the romantic/pun theme ("locked away").

**Chastity** protects your account credentials, passkeys, and two-factor authentication tokens. Built on industry-standard KDBX4 cryptography (KeePass compatible), Chastity safeguards your master encryption keys using the **Shibari** volatile memory organ, ensuring secrets are never swapped or written to disk.

---

## Launching Chastity

- **From Spindle**:
  ```text
  run chastity                # Open the graphical credential vault
  ```
- **From Voyeur**: Press **`Super+D`**, type `chastity` or `passwords`, and press **`Enter`**.

---

## Key Features

### 1. KDBX4 Standard Encryption
Chastity stores credentials in an industry-standard KDBX4 database:
- Compatible with KeePassXC, Bitwarden, and mobile KeePass clients.
- Encrypted with AES-256 or ChaCha20 cipher engines and Argon2id key derivation.
- You can freely export or import vaults between SexOS and other operating systems.

### 2. Shibari Memory Protection
In traditional operating systems, unlocked password databases can leak secrets into swap files or unprivileged process memory dumps.
In SexOS:
- The unlocked master key resides exclusively in memory pages owned by **Shibari**.
- Memory pages are hardware-isolated by PKU keys and zeroed on close.
- When the screen locks (**`Super+Escape`**) or an idle timer expires, Chastity immediately purges keys from RAM.

### 3. Time-Based One-Time Passwords (TOTP)
Chastity includes built-in RFC 6238 two-factor authentication:
- Add TOTP secret seeds or scan authenticator URIs.
- Chastity generates real-time 6-digit verification codes beside your login credentials with countdown timers.

### 4. High-Entropy Password Generator
Generate cryptographically strong, random passwords with configurable parameters:
- Length (12–128 characters).
- Character sets: uppercase, lowercase, numbers, symbols, or memorable passphrase wordlists.

### 5. Secure Clipboard Clearing
When you copy a password or TOTP code:
- Silk marks the clipboard entry as sensitive.
- After 30 seconds, Silk automatically purges the password from clipboard memory to prevent accidental pasting into public chat or terminal windows.
