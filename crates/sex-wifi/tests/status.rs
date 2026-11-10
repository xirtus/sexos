// SPDX-License-Identifier: GPL-3.0-only
//! The desktop's Wi-Fi status words (`sex_pdx::wifi_status`), checked on the host.
//! sex-pdx's own unit tests do not build on the host, so its module's tests run here.
#[path = "../../sex-pdx/src/wifi_status.rs"]
#[allow(dead_code)]
mod wifi_status;
