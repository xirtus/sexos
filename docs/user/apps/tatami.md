# Tatami (Settings Center)

> **Identity Note**: **Tatami** (`apps/settings`, `/bin/tatami`, `crates/tatami`) is the SexOS centralized preferences and system configuration manager, named after traditional woven mats in adherence to the fabric naming theme. In Spindle, typing `tatami` or `settings` opens Tatami.

**Tatami** provides a unified graphical control panel for managing display parameters, keyboard layouts, sound devices, network interfaces, appearance themes, and security capability grants.

---

## Launching Tatami

- **From Spindle**:
  ```text
  run tatami
  ```
- **From Voyeur**: Press **`Super+D`**, type `tatami` or `settings`, and press **`Enter`**.

---

## Configuration Panels

Tatami organizes settings into intuitive, searchable categories:

### 1. Display & Screen
- **Resolution & Scaling**: Select display modes (1280×800, 1920×1080, 2560×1440, 4K) and interface scaling factors.
- **Refresh Rates**: Configure hardware scanout cadence.
- **Night Light**: Adjust color temperature warming (Off, Low, Medium, High) to reduce blue light during evening hours.

### 2. Appearance & Theming
- **Wallpaper Selection**: Choose from high-resolution bundled desktop backgrounds or set custom pictures from **[Loom](loom.md)**.
- **Color Themes**: Select dark mode, light mode, high contrast, and custom system accent highlights.
- **Font Sizing**: Globally scale UI typography across all SexOS applications.

### 3. Keyboard & Input
- **Layouts**: Switch active typing keymaps (US English, Dvorak, Colemak, Neo, Workman).
- **Typematic Rates**: Configure key repeat delay and speed.
- **Caps Lock Behavior**: Toggle Caps Lock as an additional Control key (`Caps as Ctrl`) for comfortable terminal and editor operation.

### 4. Sound & Audio
- **Master Volume**: Set overall hardware output level or mute.
- **Per-Application Mixers**: Regulate audio levels independently for individual running programs (e.g., lower **[Qupid](qupid.md)** while keeping **[Flirt](flirt.md)** voice calls clear).

### 5. Network & Connections
- **Wi-Fi Management**: Scan for 802.11 wireless access points, inspect signal strength, and connect with WPA2/WPA3 authentication. Passwords are saved directly to the **Shibari** volatile vault.
- **Ethernet**: Manage IP configuration (DHCP or static addressing), subnet routing, and DNS servers.

### 6. Privacy & Capability Grants (Collar)
The most important panel in Tatami:
- **Active Grants**: Inspect every program on your machine and see what resources it can access (Network, Filesystem, Package Store, Sound).
- **Access Logs**: Review exactly when each application last exercised its capability tokens.
- **One-Click Revocation**: Instantly revoke any application grant. When revoked, **Collar** invalidates the underlying capability token immediately across the entire single address space.

### 7. System Updates & A/B Slots
- View currently active boot slot (Slot A or Slot B).
- Check for cryptographically signed system updates.
- Test or permanently roll back to the previous stable system slot.

### 8. About & Hardware Census
Displays verified hardware specifications:
- CPU model, core topology, and PKU feature verification.
- Total system RAM and active organ memory allocations.
- Storage volumes (NVMe, AHCI) managed by **BDSM**.
- SexOS kernel version, ABI hashes, and build commit signatures.
