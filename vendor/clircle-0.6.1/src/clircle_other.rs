// SexOS: `Clircle` where files have no identity to compare (std's Hermit target, which
// SexOS programs build for). No identifier can be made, so `Identifier::stdout()` is
// `None` and a program never finds a circle, as on Windows for its terminal.
use crate::{Clircle, Stdio};

use std::convert::{Infallible, TryFrom};
use std::fs::File;
use std::io;

/// No value: an identifier is never made here.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct Identifier(Infallible);

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "files cannot be identified on this platform",
    )
}

impl Clircle for Identifier {
    fn into_inner(self) -> Option<File> {
        match self.0 {}
    }
}

impl TryFrom<Stdio> for Identifier {
    type Error = io::Error;

    fn try_from(_: Stdio) -> Result<Self, Self::Error> {
        Err(unsupported())
    }
}

impl TryFrom<File> for Identifier {
    type Error = io::Error;

    fn try_from(_: File) -> Result<Self, Self::Error> {
        Err(unsupported())
    }
}
