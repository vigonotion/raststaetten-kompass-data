//! Error type of the pipeline.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// Wrong command line; the caller prints the usage text. Exit code 2.
    Usage(String),
    Io(String, std::io::Error),
    Osm(String, osmpbf::Error),
    Json(String, serde_json::Error),
    /// Invalid input data (brands file, malformed values, ...).
    Data(String),
    /// Dataset validation failed; contains one line per error.
    Validation(Vec<String>),
    /// External command failed.
    Command(String),
}

impl Error {
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Usage(_) => 2,
            _ => 1,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(m) => write!(f, "{m}"),
            Error::Io(ctx, e) => write!(f, "{ctx}: {e}"),
            Error::Osm(ctx, e) => write!(f, "{ctx}: {e}"),
            Error::Json(ctx, e) => write!(f, "{ctx}: {e}"),
            Error::Data(m) => write!(f, "{m}"),
            Error::Validation(errs) => {
                write!(f, "validation failed with {} error(s):", errs.len())?;
                for e in errs {
                    write!(f, "\n  - {e}")?;
                }
                Ok(())
            }
            Error::Command(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
