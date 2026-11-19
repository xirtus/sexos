# sexshop (Application Store)

> **Identity Note**: **sexshop** (`apps/store`, `/bin/sexshop`) is the official application store and package manager for SexOS, named under the project's sex-pun theme (`docs/DESIGN_GUARDRAILS.md` §6).

**sexshop** is a package distribution and application discovery manager. Every package distributed through sexshop is cryptographically signed, sandboxed, and verified before installation. It provides both a graphical catalog browser and direct command-line operations in Spindle.

---

## Launching sexshop

- **Graphical Store**:
  ```text
  run +net +pkg sexshop       # Launch the visual store with network & package authority
  ```
- **From Voyeur**: Press **`Super+D`**, type `sexshop`, and press **`Enter`**.

---

## Command-Line Operations in Spindle

Spindle includes built-in commands for controlling sexshop:

```text
store refresh                 # Fetch newest signed package catalogs from trusted mirrors
store search audio            # Search packages matching "audio"
store get moan                # Download, verify cryptographic signature, and install Moan
store upgrade                 # Upgrade all installed packages to their latest signed versions
store remove oldapp           # Safely remove an application and clear its binary
pkg rollback moan             # Roll back an application immediately to its previous version
```

---

## Package Architecture & Security

### 1. Cryptographic Signatures
All SexOS packages (`.pkg`) are verified using Ed25519 signatures and SHA-256 digests. If a package has been tampered with or contains invalid checksums, sexshop refuses extraction.

### 2. First-Run Grant Review
When an application is installed and launched for the first time, sexshop and **Collar** present a clear grant request summary:
- Does this app ask for network communication?
- Does this app require microphone or camera access?
- You have complete control to approve or permanently decline any requested authority.

### 3. Per-Package A/B Rollback
Upgrades are non-destructive:
- When you upgrade an application, sexshop preserves the previous release binary in a reserved rollback directory (`/system/packages/prev/<name>`).
- If a new software version exhibits a bug or regression, running `pkg rollback <name>` restores the previous known-good binary in milliseconds without re-downloading.

### 4. Custom Third-Party Repositories
You can add third-party developer repositories by maintaining trusted mirrors in `/system/packages/repos.txt`. Each repository maintains its own verified signing keys, preventing cross-repository package hijacking.
