# Mesh (System Overview & Capability Graph)

> **Identity Note**: **Mesh** (`apps/monitor`, `/bin/mesh`) is the SexOS system overview, resource monitor, and capability graph inspector, named under the fabric theme for the intricate mesh of capabilities connecting system organs.

**Mesh** provides real-time visibility into the inner workings of the SexOS microkernel. It visualizes per-core CPU utilization, physical memory allocation, hardware protection domains (PKU), active server organs, and the live capability grant topology.

---

## Launching Mesh

- **From Spindle**:
  ```text
  run mesh
  ```
- **From Voyeur**: Press **`Super+D`**, type `mesh`, and press **`Enter`**.

---

## Views & Tabs

Press **`Tab`** to switch between Mesh's primary operational views:

### 1. Protection Domains & Programs
Displays a live tabular census of all executing software:
- **Domain ID (PD)**: The hardware protection key register assignment.
- **Program Name**: The verified binary or server organ identity.
- **CPU Share**: Real-time percentage of processor time consumed.
- **Memory Footprint**: Active physical page frames allocated in RAM.
- **Status**: Running, Sleeping, Blocked on PDX, or Paused.

### 2. Live Capability Graph
The signature feature of Mesh:
- Renders an interactive directed graph showing which applications hold capability tokens to which servers and files.
- Visualizes active network channels connected to **sexnet**, open volume lenses to **BDSM**, and file custody granted by **Collar**.
- Enables instant visual verification that unprivileged apps are not leaking authority.

### 3. Hardware Telemetry & Thermals
- **Per-Core CPU Graphs**: Visual activity sparklines for each logical CPU core.
- **Memory Consumption**: Page usage breakdown across microkernel, server organs, and user programs.
- **Thermal Sensors**: Core temperature readings and fan speeds.
- **NVMe SMART**: Storage health, write endurance, and controller temperature.

---

## Managing Programs from Mesh

| Shortcut | Action | Description |
|---|---|---|
| **`Up / Down`** | **Select Domain** | Move cursor highlight through active protection domains. |
| **`K`** | **End Program** | Send graceful termination request to the selected program. |
| **`Shift+K`** | **Force Kill** | Forcibly revoke protection domain and reclaim memory immediately. |
| **`R`** | **Restart** | Restart a crashed or stopped service organ in place. |
| **`Tab`** | **Cycle View** | Switch between Processes, Capability Graph, and Hardware Telemetry. |
| **`Q` / `Esc`** | **Quit** | Close Mesh. |
