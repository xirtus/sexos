// SPDX-License-Identifier: GPL-3.0-only
//! Needle's protocols: driverless printing and scanning without CUPS or SANE.
//!
//! Everything here is bytes in, bytes out; the program (`apps/needle`) owns the
//! sockets. [`ipp`] encodes and decodes IPP/1.1-2.0 messages (RFC 8010/8011),
//! [`caps`] reads what an IPP Everywhere printer says it can do, [`raster`] writes
//! PWG Raster (PWG 5102.4) and Apple URF pages, [`pdf`] sniffs PDF and writes a
//! plain-text PDF, [`dnssd`] builds and reads DNS-SD/mDNS packets (RFC 6762/6763),
//! [`escl`] speaks to eSCL (AirScan) scanners, and [`http`] is the small HTTP/1.1
//! client both of them ride on.
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod caps;
pub mod dnssd;
pub mod escl;
pub mod http;
pub mod ipp;
pub mod pdf;
pub mod raster;
pub mod url;
pub mod xml;
