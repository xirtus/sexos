// SPDX-License-Identifier: GPL-3.0-only
//! Steamy, the weather card's helper (slice S5c): a hidden program the Shell
//! runs with the network right while the desktop widgets show. The Shell tells
//! it the time zone (`sex_pdx::desktop_widgets::OP_WEATHER_PLACE`); it asks
//! Open-Meteo (no account, no key) about that zone's city over HTTPS, and sends
//! the Shell a `Report` (`OP_WEATHER`): today's temperature, high, low and sky,
//! or why there is none. It asks again every half hour, five minutes after a
//! failure, and at once when the zone changes. It holds no files and sends
//! nothing but the city's rounded coordinates.
//!
//! Serial record: `[steamy.ready]`, `[steamy.ask] zone=.. lat=.. lon=..`,
//! `[steamy.fail] zone=.. step=connect|tls|send|read|status|parse`,
//! `[steamy.report] zone=.. report=.. ms=.. sent=0|1`.
use sex_std as _;

#[cfg(target_os = "hermit")]
mod helper {
    use sex_std::sex_app::mailbox;
    use sex_std::sex_pdx::{self as pdx, app_lifecycle as lifecycle, desktop_widgets as dw, program_store as store, PdxMessage};
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::os::hermit::io::AsRawFd;
    use std::time::Duration;

    const SHELL: u32 = 3;
    const HOST: &str = "api.open-meteo.com";
    /// Ticks (milliseconds) until the next look: after an answer, after a failure.
    const FRESH: u64 = 30 * 60 * 1000;
    const RETRY: u64 = 5 * 60 * 1000;
    /// Largest answer kept (Open-Meteo's is under 1 KB).
    const MAX: usize = 16 * 1024;

    struct Failed(&'static str);

    fn get(path: &[u8]) -> Result<Vec<u8>, Failed> {
        let mut s = TcpStream::connect((HOST, 443)).map_err(|_| Failed("connect"))?;
        let _ = s.set_read_timeout(Some(Duration::from_secs(20)));
        let _ = s.set_write_timeout(Some(Duration::from_secs(20)));
        // TLS on the connected socket: sexnet checks the certificate for HOST.
        if unsafe { sex_std::sys::sys_tls_start(s.as_raw_fd(), HOST.as_ptr(), HOST.len()) } != 0 { return Err(Failed("tls")); }
        let mut req = Vec::with_capacity(320);
        req.extend_from_slice(b"GET ");
        req.extend_from_slice(path);
        req.extend_from_slice(b" HTTP/1.0\r\nHost: ");
        req.extend_from_slice(HOST.as_bytes());
        req.extend_from_slice(b"\r\nUser-Agent: SexOS\r\nAccept: application/json\r\nConnection: close\r\n\r\n");
        s.write_all(&req).map_err(|_| Failed("send"))?;
        let mut got = Vec::new();
        let mut buf = [0u8; 2048];
        loop {
            match s.read(&mut buf) {
                Ok(0) => break,
                Ok(n) if got.len() + n <= MAX => got.extend_from_slice(&buf[..n]),
                _ => return Err(Failed("read")),
            }
        }
        let head = got.windows(4).position(|w| w == b"\r\n\r\n").ok_or(Failed("read"))?;
        let ok = got.starts_with(b"HTTP/1.") && got.get(8..13) == Some(b" 200 ".as_slice());
        if !ok { return Err(Failed("status")); }
        Ok(got[head + 4..].to_vec())
    }

    fn ask(zone: u64) -> dw::Report {
        let name = sex_config::ZONES.get(zone as usize).copied().unwrap_or("UTC");
        let Some((lat, lon)) = dw::place(name) else { return dw::Report::Unavailable(dw::NO_PLACE) };
        pdx::serial_record!("[steamy.ask] zone={} lat={} lon={}", zone, lat, lon);
        let mut path = [0u8; 192];
        let n = dw::forecast_path(lat, lon, &mut path);
        match get(&path[..n]) {
            Ok(body) => match dw::parse_forecast(&body) {
                Some(w) => dw::Report::Ok(w),
                None => { pdx::serial_record!("[steamy.fail] zone={} step=parse", zone); dw::Report::Unavailable(dw::NO_FORECAST) }
            },
            Err(Failed(step)) => {
                pdx::serial_record!("[steamy.fail] zone={} step={}", zone, step);
                if step == "status" { dw::Report::Unavailable(dw::NO_FORECAST) } else { dw::Report::Unavailable(dw::NO_NETWORK) }
            }
        }
    }

    fn tell_shell(report: dw::Report, zone: u64) -> bool {
        for _ in 0..1000 {
            match pdx::pdx_call_checked(pdx::SLOT_SHELL, dw::OP_WEATHER, report.pack(), zone, 0) {
                Ok(_) => return true,
                Err(pdx::ERR_SERVICE_NOT_READY) => pdx::sys_yield(),
                Err(_) => return false,
            }
        }
        false
    }

    enum Said { Zone(u64), Quit }

    /// What the Shell said since the last look, without waiting. Messages for
    /// other tasks go to the mailbox for their owner.
    fn listen(me: (u32, u32)) -> Option<Said> {
        let ours = |m: &PdxMessage| m.caller_pd == SHELL;
        let said = |m: PdxMessage| {
            if m.type_id == dw::OP_WEATHER_PLACE { Some(Said::Zone(m.arg0)) }
            else if lifecycle::close_requested(m.type_id, m.caller_pd, [m.arg0, m.arg1, m.arg2], me.0, me.1) { Some(Said::Quit) }
            else { None }
        };
        let mut last = None;
        while let Some(m) = mailbox::take(ours) { if let Some(s) = said(m) { last = Some(s); if matches!(last, Some(Said::Quit)) { return last; } } }
        while let Some(m) = pdx::pdx_try_listen_raw(0) {
            if !ours(&m) { mailbox::put(m); continue; }
            if let Some(s) = said(m) { last = Some(s); if matches!(last, Some(Said::Quit)) { return last; } }
        }
        if store::record_call(store::RECORD_STOP_REQUESTED, 0, 0).1 != 0 { return Some(Said::Quit); }
        last
    }

    pub fn serve() {
        let me = sex_std::identity();
        pdx::serial_record!("[steamy.ready] target={} task={}", me.0, me.1);
        let mut zone: Option<u64> = None;
        let mut due = 0u64;
        loop {
            match listen(me) {
                Some(Said::Quit) => return,
                Some(Said::Zone(z)) if zone != Some(z) => { zone = Some(z); due = 0; }
                _ => {}
            }
            let now = pdx::get_ticks();
            if let Some(z) = zone.filter(|_| now >= due) {
                let report = ask(z);
                let ms = pdx::get_ticks().saturating_sub(now);
                // The zone may have changed while asking: the next turn asks again.
                let sent = tell_shell(report, z);
                pdx::serial_record!("[steamy.report] zone={} report={:?} ms={} sent={}", z, report, ms, sent as u8);
                due = pdx::get_ticks() + if matches!(report, dw::Report::Ok(_)) { FRESH } else { RETRY };
                continue;
            }
            // Sleep until the next look; a message from the Shell wakes it sooner.
            let left = if zone.is_some() { due.saturating_sub(now) } else { 60_000 };
            let _ = pdx::sys_wait_self_ticks(left.clamp(1, 60_000));
        }
    }
}

fn main() {
    #[cfg(target_os = "hermit")]
    helper::serve();
    #[cfg(not(target_os = "hermit"))]
    println!("steamy runs on SexOS, started by the Shell for the weather card");
}
