# Communication & Sharing Suite

SexOS provides an integrated ecosystem for online communication, decentralized messaging, smartphone synchronization, and peer-to-peer sharing. Each component is strictly sandboxed by **Collar** and named under the project's canonical themes.

---

## Messaging & Unified Inbox

| Application | Conceptual Role | Theme & Heritage | Description | User Manual |
|---|---|---|---|---|
| **Valentine** | Mail & Chat Hub | Romance (a love letter; Andreas's pick) | Unified communications hub uniting traditional email (IMAP/SMTP) and modern encrypted messaging (Matrix, Mumble) into a single cohesive inbox. | *Suite Hub* |
| **Flirt** | Voice & Video Calls | Romance (Andreas's pick; replaces Talk) | Low-latency audio calls, video conferences, and desktop screen sharing with WebRTC and peer-to-peer channels. | **[Flirt Manual](apps/flirt.md)** |
| **Multipass** | Online Accounts & Identity | Identity / Sci-Fi (Andreas's Leeloo Multipass) | Centralized authentication and identity manager for Google, Nextcloud, GitHub, and federated Matrix/Fediverse accounts. | *Suite Tool* |
| **Crush** | Contacts & Address Book | Romance ("the people you keep") | Contact management with phone numbers, cryptographic public keys, email addresses, and communication history. | *Suite Tool* |

---

## Hardware Linking & Peripherals

| Application | Conceptual Role | Theme & Heritage | Description | User Manual |
|---|---|---|---|---|
| **Hookup** | Phone Link | Romance / Pun (PR #239) | Smartphone companion utilizing the KDE Connect protocol: receive phone notifications on SilkBar, reply to SMS, share clipboards, and transfer photos. | *Suite Tool* |
| **Cufflinks** | Bluetooth Manager | Fashion / Pairing (goes with Tuxedo, PR #234) | Native Bluetooth device discovery, audio headset pairing, low-energy HID input linking, and file beaming. | *Suite Tool* |
| **Needle** | Printing & Scanning | Fabric / Free word (PR #233) | Driverless IPP (Internet Printing Protocol) and eSCL scanner manager supporting modern network and USB printers. | *Suite Tool* |

---

## Navigation & Remote Access

| Application | Conceptual Role | Theme & Heritage | Description | User Manual |
|---|---|---|---|---|
| **Rendezvous** | Maps & Navigation | Romance ("where you meet", PR #239) | Geospatial map viewer and routing tool powered by OpenStreetMap with offline vector tile caching. | **[Rendezvous Manual](apps/rendezvous.md)** |
| **Lurk** | Remote Desktop | Observation / Free word (PR #226) | Remote desktop client supporting VNC and RDP protocols with encrypted tunneling. | *Suite Tool* |
| **Swingers** | BitTorrent Client | Pun ("everyone swaps with everyone", PR #236) | High-speed, decentralized BitTorrent client engine (`sex-torrent`) with magnet link handling and bandwidth throttling via **Choke**. | *Suite Tool* |

---

## Security & Network Confinement

Communication applications in SexOS hold only the specific network endpoints they require:
- **Zero Ambient Network Access**: Programs cannot open random listening sockets without being granted network authority (`+net`).
- **Encrypted Credentials in Shibari**: Account passwords, OAuth access tokens, and private SSH/PGP keys are stored exclusively in **Shibari** volatile memory pages and are never exposed in unencrypted filesystem logs.
- **Bell Integration**: Messages arriving in **Valentine** or incoming phone notifications from **Hookup** generate native banners via **Bell** (`Super+N`).
