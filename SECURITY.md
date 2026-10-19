# Security policy

SexOS is an experimental operating system. It has not received a production
security audit, and no version is supported for production use.

## Reporting a vulnerability

Please report security issues privately through GitHub Security Advisories on
this repository. Do not open a public issue containing exploit details, kernel
addresses, or a working proof of concept.

Include:

- the commit/ISO version,
- the QEMU command and host environment,
- a minimal reproducer or description,
- the impact you believe it has.

## Scope

In scope: the kernel, `sex-pdx`, the in-tree servers, the build scripts, and the
ISO/QEMU boot path. Out of scope: third-party firmware, host QEMU bugs, and
hardware-specific issues that cannot be reproduced in QEMU.

## Supported versions

None. The latest `main` is the only branch under active development.
