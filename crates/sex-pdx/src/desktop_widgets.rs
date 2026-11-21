// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Desktop widgets (workstream S5): cards drawn on the wallpaper, under every
//! window, the way Plasma's desktop widgets and GNOME's Desktop Clock
//! extension sit. Display draws them as part of the wallpaper, so a window
//! covers them and nothing about them can take focus or input.
//!
//! The Shell turns them on or off (setting `desktop.widgets`, Super+Shift+W)
//! with `OP_DESKTOP_WIDGETS`. Display fills them itself: the clock from the
//! civil time and zone it already keeps for the bar, the system card from the
//! kernel's numbers (syscall 87, which needs no right). The weather card's
//! numbers come from a hidden helper program (`apps/steamy`, with the network
//! right) that asks Open-Meteo about the time zone's city; the Shell starts it,
//! tells it the zone ([`OP_WEATHER_PLACE`]) and passes its [`Report`]s on to
//! Display ([`OP_WEATHER`]).
//!
//! The words and the layout are here, host-tested in `tests/desktop_widgets.rs`.

/// Shell -> Display. arg0 = which cards show ([`CLOCK`] | [`SYSTEM`] | [`WEATHER`]); 0 hides all.
pub const OP_DESKTOP_WIDGETS: u64 = 0x2F9;
/// Helper -> Shell, and Shell -> Display: arg0 = [`Report::pack`], arg1 = the
/// zone index (`sex_config::ZONES` order) the report is for.
pub const OP_WEATHER: u64 = 0x2FA;
/// Shell -> helper: arg0 = the zone index to ask about.
pub const OP_WEATHER_PLACE: u64 = 0x2FB;
pub const CLOCK: u64 = 1;
pub const SYSTEM: u64 = 2;
/// The weather card (Open-Meteo numbers carried by the Shell).
pub const WEATHER: u64 = 4;
pub const ALL: u64 = CLOCK | SYSTEM | WEATHER;

/// A line of card text.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Line { pub bytes: [u8; 40], pub len: usize }

impl Line {
    pub const EMPTY: Line = Line { bytes: [0; 40], len: 0 };
    pub fn as_bytes(&self) -> &[u8] { &self.bytes[..self.len] }
    fn push(&mut self, s: &[u8]) { for &b in s { if self.len < self.bytes.len() { self.bytes[self.len] = b; self.len += 1; } } }
    fn num(&mut self, mut v: u64, width: usize) {
        let mut d = [0u8; 20];
        let mut n = 0;
        loop { d[n] = b'0' + (v % 10) as u8; n += 1; v /= 10; if v == 0 { break; } }
        while n < width { d[n] = b'0'; n += 1; }
        for i in (0..n).rev() { self.push(&[d[i]]); }
    }
}

impl core::fmt::Debug for Line {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", core::str::from_utf8(self.as_bytes()).unwrap_or("?"))
    }
}

pub const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
pub const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// Year, month (1..=12) and day of a day count since 1970-01-01 (civil, proleptic Gregorian).
pub fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

/// The clock card for `local` seconds since 1970 in the person's zone:
/// ("14:05", "Tuesday 17 November"), or "2:05 PM" without the 24-hour clock.
pub fn clock_lines(local: i64, hour24: bool) -> (Line, Line) {
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400) as u64;
    let (h, m) = (secs / 3600, secs / 60 % 60);
    let mut time = Line::EMPTY;
    if hour24 { time.num(h, 2); } else { time.num(if h % 12 == 0 { 12 } else { h % 12 }, 1); }
    time.push(b":");
    time.num(m, 2);
    if !hour24 { time.push(if h < 12 { b" AM" } else { b" PM" }); }
    let (_, month, day) = civil_date(days);
    let mut date = Line::EMPTY;
    // 1970-01-01 was a Thursday.
    date.push(WEEKDAYS[(days + 3).rem_euclid(7) as usize].as_bytes());
    date.push(b" ");
    date.num(day as u64, 1);
    date.push(b" ");
    date.push(MONTHS[month as usize - 1].as_bytes());
    (time, date)
}

/// One look at the kernel's overview: TSC, cores, idle cycles of all cores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample { pub tsc: u64, pub cores: u64, pub idle: u64 }

/// How busy the processors were between two samples, in percent.
pub fn cpu_percent(before: Sample, now: Sample) -> Option<u8> {
    let span = now.tsc.checked_sub(before.tsc)?.checked_mul(now.cores.max(1))?;
    if span == 0 { return None; }
    let idle = now.idle.checked_sub(before.idle)?.min(span);
    Some((((span - idle) as u128 * 100 + span as u128 / 2) / span as u128).min(100) as u8)
}

/// Bytes as people read them: "512 MB", "1.2 GB".
pub fn size(bytes: u64, out: &mut Line) {
    const MB: u64 = 1 << 20;
    const GB: u64 = 1 << 30;
    if bytes >= GB {
        let tenths = (bytes * 10 + GB / 2) / GB;
        out.num(tenths / 10, 1);
        out.push(b".");
        out.num(tenths % 10, 1);
        out.push(b" GB");
    } else {
        out.num((bytes + MB / 2) / MB, 1);
        out.push(b" MB");
    }
}

/// The system card: ("Processor 12%", "Memory free 1.2 GB"); a dash while unknown.
pub fn system_lines(cpu: Option<u8>, free_pages: Option<u64>) -> (Line, Line) {
    let mut a = Line::EMPTY;
    a.push(b"Processor ");
    match cpu { Some(p) => { a.num(p as u64, 1); a.push(b"%"); } None => a.push(b"-") }
    let mut b = Line::EMPTY;
    b.push(b"Memory free ");
    match free_pages { Some(p) => size(p.saturating_mul(4096), &mut b), None => b.push(b"-") }
    (a, b)
}

/// Where the cards go on a `w`x`h` screen under a `bar`-high bar, at UI
/// `scale`: a column at the top right. (x, y, w, h) of the column and of
/// each shown card, clock, weather then system from the top; None when
/// nothing shows or the screen is too small.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub column: (usize, usize, usize, usize),
    pub clock: Option<(usize, usize, usize, usize)>,
    pub weather: Option<(usize, usize, usize, usize)>,
    pub system: Option<(usize, usize, usize, usize)>,
    pub scale: usize,
}

pub const CARD_W: usize = 240;
pub const CLOCK_H: usize = 104;
pub const SYSTEM_H: usize = 64;
pub const WEATHER_H: usize = 64;
pub const GAP: usize = 12;
pub const MARGIN: usize = 24;

impl Layout {
    pub fn new(cards: u64, w: usize, h: usize, bar: usize, scale: usize) -> Option<Layout> {
        let s = scale.clamp(1, 2);
        let shown = [(cards & CLOCK != 0, CLOCK_H), (cards & WEATHER != 0, WEATHER_H), (cards & SYSTEM != 0, SYSTEM_H)];
        let n = shown.iter().filter(|c| c.0).count();
        if n == 0 { return None; }
        let height = shown.iter().filter(|c| c.0).map(|c| c.1 * s).sum::<usize>() + GAP * s * (n - 1);
        let (cw, m) = (CARD_W * s, MARGIN * s);
        if w < cw + 2 * m || h < bar + height + 2 * m { return None; }
        let (x, mut y) = (w - m - cw, bar + m);
        let mut place = |on: bool, ch: usize| on.then(|| { let r = (x, y, cw, ch * s); y += ch * s + GAP * s; r });
        let clock = place(shown[0].0, CLOCK_H);
        let weather = place(shown[1].0, WEATHER_H);
        let system = place(shown[2].0, SYSTEM_H);
        Some(Layout { column: (x, bar + m, cw, height), clock, weather, system, scale: s })
    }
}

/// Today's weather: degrees Celsius in tenths, and the WMO weather code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Weather { pub now: i16, pub high: i16, pub low: i16, pub code: u8 }

/// Why there is no weather to show.
pub const NO_PLACE: u8 = 1;
pub const NO_NETWORK: u8 = 2;
pub const NO_FORECAST: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report { Ok(Weather), Unavailable(u8) }

impl Report {
    /// kind (8 bits) | code or reason (8) | now (16) | high (16) | low (16).
    pub fn pack(self) -> u64 {
        match self {
            Report::Ok(w) => 1 << 56 | (w.code as u64) << 48 | (w.now as u16 as u64) << 32 | (w.high as u16 as u64) << 16 | w.low as u16 as u64,
            Report::Unavailable(why) => 2 << 56 | (why as u64) << 48,
        }
    }
    pub fn unpack(v: u64) -> Option<Report> {
        let byte = (v >> 48) as u8;
        match v >> 56 {
            1 => Some(Report::Ok(Weather { now: (v >> 32) as u16 as i16, high: (v >> 16) as u16 as i16, low: v as u16 as i16, code: byte })),
            2 if (NO_PLACE..=NO_FORECAST).contains(&byte) && v & 0xFFFF_FFFF_FFFF == 0 => Some(Report::Unavailable(byte)),
            _ => None,
        }
    }
}

/// Where each time zone's city is, in hundredths of a degree (latitude,
/// longitude): the place the weather card asks about. UTC has none.
pub const PLACES: &[(&str, i16, i16)] = &[
    ("Europe/London", 5151, -13), ("Europe/Dublin", 5335, -626), ("Europe/Lisbon", 3872, -914),
    ("Europe/Paris", 4886, 235), ("Europe/Berlin", 5252, 1340), ("Europe/Madrid", 4042, -370),
    ("Europe/Rome", 4190, 1250), ("Europe/Amsterdam", 5237, 490), ("Europe/Stockholm", 5933, 1807),
    ("Europe/Oslo", 5991, 1075), ("Europe/Copenhagen", 5568, 1257), ("Europe/Helsinki", 6017, 2494),
    ("Europe/Warsaw", 5223, 2101), ("Europe/Prague", 5008, 1444), ("Europe/Vienna", 4821, 1637),
    ("Europe/Zurich", 4738, 854), ("Europe/Athens", 3798, 2373), ("Europe/Istanbul", 4101, 2898),
    ("Europe/Kyiv", 5045, 3052), ("Europe/Moscow", 5576, 3762),
    ("America/New_York", 4071, -7401), ("America/Chicago", 4188, -8763), ("America/Denver", 3974, -10499),
    ("America/Phoenix", 3345, -11207), ("America/Los_Angeles", 3405, -11824), ("America/Anchorage", 6122, -14990),
    ("Pacific/Honolulu", 2131, -15786), ("America/Toronto", 4365, -7938), ("America/Vancouver", 4928, -12312),
    ("America/Mexico_City", 1943, -9913), ("America/Sao_Paulo", -2355, -4663), ("America/Buenos_Aires", -3460, -5838),
    ("America/Bogota", 471, -7407), ("America/Santiago", -3345, -7067),
    ("Asia/Tokyo", 3568, 13969), ("Asia/Seoul", 3757, 12698), ("Asia/Shanghai", 3123, 12147),
    ("Asia/Hong_Kong", 2232, 11417), ("Asia/Taipei", 2503, 12157), ("Asia/Singapore", 135, 10382),
    ("Asia/Kolkata", 2257, 8836), ("Asia/Dubai", 2520, 5527), ("Asia/Bangkok", 1376, 10050),
    ("Asia/Jakarta", -621, 10685), ("Asia/Manila", 1460, 12098), ("Asia/Karachi", 2486, 6700),
    ("Asia/Tehran", 3569, 5139), ("Asia/Jerusalem", 3177, 3521),
    ("Africa/Cairo", 3004, 3124), ("Africa/Johannesburg", -2620, 2805), ("Africa/Lagos", 652, 338),
    ("Africa/Nairobi", -129, 3682),
    ("Australia/Sydney", -3387, 15121), ("Australia/Melbourne", -3781, 14496), ("Australia/Brisbane", -2747, 15303),
    ("Australia/Perth", -3195, 11586), ("Australia/Adelaide", -3493, 13860), ("Pacific/Auckland", -3685, 17476),
];

/// The zone's city: (latitude, longitude) in hundredths of a degree.
pub fn place(zone: &str) -> Option<(i16, i16)> {
    PLACES.iter().find(|p| p.0 == zone).map(|p| (p.1, p.2))
}

/// The city a zone is named after: "America/New_York" -> "New York".
pub fn city(zone: &str, out: &mut Line) {
    for &b in zone.rsplit('/').next().unwrap_or(zone).as_bytes() { out.push(&[if b == b'_' { b' ' } else { b }]); }
}

/// Zones whose people read Fahrenheit.
pub fn fahrenheit(zone: &str) -> bool {
    matches!(zone, "America/New_York" | "America/Chicago" | "America/Denver" | "America/Phoenix"
        | "America/Los_Angeles" | "America/Anchorage" | "Pacific/Honolulu")
}

/// Open-Meteo's forecast path for a place: the current temperature and
/// weather code, and today's high and low, in Celsius. Its length in `out`.
pub fn forecast_path(lat: i16, lon: i16, out: &mut [u8; 192]) -> usize {
    let mut n = 0;
    let mut put = |s: &[u8], n: &mut usize| { for &b in s { if *n < out.len() { out[*n] = b; *n += 1; } } };
    let degrees = |v: i16| {
        let mut p = Line::EMPTY;
        if v < 0 { p.push(b"-"); }
        let a = v.unsigned_abs() as u64;
        p.num(a / 100, 1); p.push(b"."); p.num(a % 100, 2);
        p
    };
    put(b"/v1/forecast?latitude=", &mut n);
    put(degrees(lat).as_bytes(), &mut n);
    put(b"&longitude=", &mut n);
    put(degrees(lon).as_bytes(), &mut n);
    put(b"&current=temperature_2m,weather_code&daily=temperature_2m_max,temperature_2m_min&forecast_days=1&timezone=auto", &mut n);
    n
}

/// The number after `"key":` (or `"key":[`) inside the object that follows
/// `"object":{`, in tenths, rounded.
fn field(body: &[u8], object: &[u8], key: &[u8]) -> Option<i32> {
    let at = find(body, object)? + object.len();
    let body = &body[at..];
    let end = body.iter().position(|&b| b == b'}').unwrap_or(body.len());
    let body = &body[..end];
    let mut k = 0;
    // `"key":` exactly, not a longer key that starts the same.
    let rest = loop {
        let i = find(&body[k..], key)? + k;
        let after = &body[i + key.len()..];
        if after.first() == Some(&b':') && i > 0 && body[i - 1] == b'"' { break &after[1..]; }
        k = i + 1;
    };
    let rest = rest.strip_prefix(b"[").unwrap_or(rest);
    let (neg, rest) = match rest.strip_prefix(b"-") { Some(r) => (true, r), None => (false, rest) };
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || digits > 5 { return None; }
    let mut v: i32 = 0;
    for &d in &rest[..digits] { v = v * 10 + (d - b'0') as i32; }
    let mut tenths = v * 10;
    if rest.get(digits) == Some(&b'.') {
        let frac = &rest[digits + 1..];
        let t = frac.first().filter(|b| b.is_ascii_digit()).map(|b| (b - b'0') as i32).unwrap_or(0);
        let up = frac.get(1).is_some_and(|b| b.is_ascii_digit() && *b >= b'5');
        tenths += t + up as i32;
    }
    Some(if neg { -tenths } else { tenths })
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() { return None; }
    (0..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// Today's weather from Open-Meteo's answer to [`forecast_path`]. None when
/// the answer lacks a number or a number is out of this world.
pub fn parse_forecast(body: &[u8]) -> Option<Weather> {
    let now = field(body, b"\"current\":{", b"temperature_2m\"")?;
    let code = field(body, b"\"current\":{", b"weather_code\"")?;
    let high = field(body, b"\"daily\":{", b"temperature_2m_max\"")?;
    let low = field(body, b"\"daily\":{", b"temperature_2m_min\"")?;
    let sane = |t: i32| (-1000..=700).contains(&t);
    if !sane(now) || !sane(high) || !sane(low) || !(0..=990).contains(&code) || code % 10 != 0 { return None; }
    Some(Weather { now: now as i16, high: high as i16, low: low as i16, code: (code / 10) as u8 })
}

/// A WMO weather code in words.
pub fn sky(code: u8) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mostly clear",
        2 => "Partly cloudy",
        3 => "Cloudy",
        45 | 48 => "Fog",
        51..=55 => "Drizzle",
        56 | 57 => "Freezing drizzle",
        61..=65 => "Rain",
        66 | 67 => "Freezing rain",
        71..=75 | 77 => "Snow",
        80..=82 => "Showers",
        85 | 86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm, hail",
        _ => "Weather",
    }
}

/// Whole degrees, in Celsius or Fahrenheit, from tenths of a degree Celsius.
fn degrees(tenths: i16, f: bool, out: &mut Line) {
    let t = tenths as i32;
    let t = if f { t * 9 / 5 + 320 } else { t };
    let whole = if t < 0 { -((-t + 5) / 10) } else { (t + 5) / 10 };
    if whole < 0 { out.push("\u{2212}".as_bytes()); }
    out.num(whole.unsigned_abs() as u64, 1);
    out.push("\u{b0}".as_bytes());
}

/// The weather card for `zone`: ("Berlin 7°", "Cloudy, 9° / 3°"), or why
/// there is none; `None`: no report yet.
pub fn weather_lines(report: Option<Report>, zone: &str) -> (Line, Line) {
    let f = fahrenheit(zone);
    let mut a = Line::EMPTY;
    let mut b = Line::EMPTY;
    match report {
        Some(Report::Ok(w)) => {
            city(zone, &mut a);
            a.push(b" ");
            degrees(w.now, f, &mut a);
            b.push(sky(w.code).as_bytes());
            b.push(b", ");
            degrees(w.high, f, &mut b);
            b.push(b" / ");
            degrees(w.low, f, &mut b);
        }
        other => {
            a.push(b"Weather");
            b.push(match other {
                None => b"Looking it up...".as_slice(),
                Some(Report::Unavailable(NO_PLACE)) => b"Set a time zone city".as_slice(),
                Some(Report::Unavailable(NO_NETWORK)) => b"Open-Meteo out of reach".as_slice(),
                _ => b"No forecast just now".as_slice(),
            });
        }
    }
    (a, b)
}
