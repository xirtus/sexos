// SPDX-License-Identifier: GPL-3.0-only
//! Dimension-checked units. The syntax is `VALUE FROM to TO`, e.g. `32 F to C`.
//! Temperature conversions include offsets; physical absolute zero is enforced.
#[derive(Clone, Copy)]
struct Unit {
    symbol: &'static str,
    dimension: u8,
    scale: f64,
    offset: f64,
}
const UNITS: &[Unit] = &[
    Unit {
        symbol: "m",
        dimension: 0,
        scale: 1.,
        offset: 0.,
    },
    Unit {
        symbol: "km",
        dimension: 0,
        scale: 1000.,
        offset: 0.,
    },
    Unit {
        symbol: "cm",
        dimension: 0,
        scale: 0.01,
        offset: 0.,
    },
    Unit {
        symbol: "mm",
        dimension: 0,
        scale: 0.001,
        offset: 0.,
    },
    Unit {
        symbol: "in",
        dimension: 0,
        scale: 0.0254,
        offset: 0.,
    },
    Unit {
        symbol: "ft",
        dimension: 0,
        scale: 0.3048,
        offset: 0.,
    },
    Unit {
        symbol: "mi",
        dimension: 0,
        scale: 1609.344,
        offset: 0.,
    },
    Unit {
        symbol: "kg",
        dimension: 1,
        scale: 1.,
        offset: 0.,
    },
    Unit {
        symbol: "g",
        dimension: 1,
        scale: 0.001,
        offset: 0.,
    },
    Unit {
        symbol: "lb",
        dimension: 1,
        scale: 0.45359237,
        offset: 0.,
    },
    Unit {
        symbol: "oz",
        dimension: 1,
        scale: 0.028349523125,
        offset: 0.,
    },
    Unit {
        symbol: "s",
        dimension: 2,
        scale: 1.,
        offset: 0.,
    },
    Unit {
        symbol: "ms",
        dimension: 2,
        scale: 0.001,
        offset: 0.,
    },
    Unit {
        symbol: "min",
        dimension: 2,
        scale: 60.,
        offset: 0.,
    },
    Unit {
        symbol: "h",
        dimension: 2,
        scale: 3600.,
        offset: 0.,
    },
    Unit {
        symbol: "day",
        dimension: 2,
        scale: 86400.,
        offset: 0.,
    },
    Unit {
        symbol: "K",
        dimension: 3,
        scale: 1.,
        offset: 0.,
    },
    Unit {
        symbol: "C",
        dimension: 3,
        scale: 1.,
        offset: 273.15,
    },
    Unit {
        symbol: "F",
        dimension: 3,
        scale: 5. / 9.,
        offset: 255.3722222222222,
    },
    Unit {
        symbol: "B",
        dimension: 4,
        scale: 1.,
        offset: 0.,
    },
    Unit {
        symbol: "kB",
        dimension: 4,
        scale: 1000.,
        offset: 0.,
    },
    Unit {
        symbol: "MB",
        dimension: 4,
        scale: 1000000.,
        offset: 0.,
    },
    Unit {
        symbol: "GB",
        dimension: 4,
        scale: 1000000000.,
        offset: 0.,
    },
    Unit {
        symbol: "KiB",
        dimension: 4,
        scale: 1024.,
        offset: 0.,
    },
    Unit {
        symbol: "MiB",
        dimension: 4,
        scale: 1048576.,
        offset: 0.,
    },
    Unit {
        symbol: "GiB",
        dimension: 4,
        scale: 1073741824.,
        offset: 0.,
    },
];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Syntax,
    Unknown,
    Dimension,
    Range,
}
pub fn convert(value: f64, from: &str, to: &str) -> Result<f64, Error> {
    if !value.is_finite() {
        return Err(Error::Range);
    }
    let from = UNITS
        .iter()
        .find(|u| u.symbol == from)
        .ok_or(Error::Unknown)?;
    let to = UNITS
        .iter()
        .find(|u| u.symbol == to)
        .ok_or(Error::Unknown)?;
    if from.dimension != to.dimension {
        return Err(Error::Dimension);
    }
    let base = value * from.scale + from.offset;
    if from.dimension == 3 && base < -1e-10 {
        return Err(Error::Range);
    }
    let value = (base - to.offset) / to.scale;
    if !value.is_finite() {
        return Err(Error::Range);
    }
    Ok(if value.abs() < 1e-12 { 0. } else { value })
}
pub fn eval(text: &str) -> Result<alloc::string::String, Error> {
    let mut words = text.split_whitespace();
    let value = words
        .next()
        .ok_or(Error::Syntax)?
        .parse::<f64>()
        .map_err(|_| Error::Syntax)?;
    let from = words.next().ok_or(Error::Syntax)?;
    if words.next() != Some("to") {
        return Err(Error::Syntax);
    }
    let to = words.next().ok_or(Error::Syntax)?;
    if words.next().is_some() {
        return Err(Error::Syntax);
    }
    Ok(alloc::format!(
        "{} {to}",
        crate::calc::format(convert(value, from, to)?)
    ))
}
