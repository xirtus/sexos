# Date: first stable reminder identity

- Branch: `codex/date-notice-dedupe`, stacked on `codex/date-presentation-ack`.
- UID-backed reminder occurrences now expose a semantic key composed of the
  source calendar, length-framed iCalendar UID, occurrence day and due UTC
  minute. It is independent of event vector order and changes for distinct
  occurrences or reminder offsets. A focused host regression covers this.
- Bell and Shell have no durable receipt store or semantic-key transport yet.
  This slice does not suppress retries, change the at-least-once guarantee, or
  claim cross-restart deduplication. Legacy UID-less events, Clock alarms and
  timers require persistent source identities before they can join that path.
- Full build, QEMU and screenshot remain GitHub CI work; no local full build or
  QEMU artifacts are produced.
