# Date (Calendar & Clocks)

> **Identity Note**: **Date** (`apps/date`, function key **F8**) is the SexOS calendar, event scheduling, and civil timekeeper application, named under the romance theme (Andreas's pick).

**Date** provides comprehensive personal time management: multi-view calendars, event scheduling, iCalendar (`.ics`) importing and exporting, CalDAV network synchronization, alarms, timers, and world clocks.

---

## Launching Date

- **Global Shortcut**: Press **`F8`** to summon the Date calendar window.
- **From Spindle**:
  ```text
  run date
  run +net date             # With network grant for CalDAV synchronization
  ```
- **From Voyeur**: Press **`Super+D`**, type `date` or `calendar`, and press **`Enter`**.

---

## Key Views & Features

Press **`Tab`** in Date to navigate between views:

### 1. Calendar & Events (Month, Week, Day)
- **Month Grid**: Visual overview of scheduled appointments and meetings.
- **Week & Day Timelines**: Hourly breakdown of daily tasks and time blocks.
- **Navigation**: Use **`Left / Right`** arrows to flip months; press **`T`** to jump immediately back to today.

### 2. Event Creation & iCalendar (`.ics`) Import
- Press **`N`** to schedule a new event: configure title, location, start/end timestamps, and recurrence rules (RRULE).
- Drag any `.ics` invite file from **[Loom](loom.md)** or **[Valentine](../communication-and-sharing.md)** directly into Date to import appointments automatically.

### 3. CalDAV Synchronization
With a network grant (`+net`), Date connects to remote CalDAV calendar servers (Nextcloud, Google Calendar, Fastmail, Apple iCloud) over TLS. Event updates sync bidirectionally in the background.

### 4. Alarms, Timers & World Clock
- **Alarms**: Set recurring or one-time alerts with sound chimes routed through **Bell**.
- **Countdown Timers & Stopwatch**: Precision countdowns with desktop notifications.
- **World Clock**: Track concurrent time across multiple international time zones beside the Silk civil clock.
