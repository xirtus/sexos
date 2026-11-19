# Licensing

SexOS is GPL-3.0-only (see `LICENSE`), except for the crates below.

## Driver-facing crates: GPL-2.0-or-later OR MIT

Decided by Andreas on 2026-11-09 so that driver services may contain code from Linux
drivers (GPL-2.0-only) and other GPL-2.0 or permissive sources. These are the crates a
driver service links:

| Crate | Path |
|---|---|
| sex-pdx | `crates/sex-pdx` |
| sex-net-config | `crates/sex-net-config` (used by sex-pdx) |
| sex-trust-config | `crates/sex-trust-config` (used by sex-pdx) |
| sex-rt | `sex-rt` |
| libsys | `sex-src/lib/libsys` |
| sex-hwdb | `crates/sex-hwdb` |
| sex-bt | `crates/sex-bt` (Bluetooth host stack core; its own dependencies `aes` and `p256` are MIT/Apache-2.0) |

You may use each of them under either the GNU General Public License, version 2 or (at
your option) any later version, or the MIT licence below. Their third-party dependencies
(`serde`, `spin`, `bitflags`, `x86_64`, `linked_list_allocator`) are MIT or MIT/Apache-2.0
dual-licensed.

A driver service that contains GPL-2.0-only code is distributed as a GPL-2.0 program. It
must then link only the crates above (or other GPL-2.0-compatible code), not GPL-3.0-only
crates. Each driver service is its own program and talks to the rest of SexOS only over
PDX, so the GPL-3.0-only services stay as they are. The same rule applies to tuxedo guests
(see `docs/handoff/DRIVERS_AND_X17_R1_PLAN_2026_11_09.md`, section 7).

When adding a crate to this list, change its `license` field and the SPDX lines of its
files to `GPL-2.0-or-later OR MIT`, and keep its dependencies inside this list.

## MIT licence text

Copyright (c) the SexOS authors

Permission is hereby granted, free of charge, to any person obtaining a copy of this
software and associated documentation files (the "Software"), to deal in the Software
without restriction, including without limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons
to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE
FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
