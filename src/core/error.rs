use std::fmt;

#[derive(Debug)]
pub enum ErrorKind {
    Io,
    Parse,
    NotFound,
    InvalidArgument,
    Internal,
    InvalidInput,
    OutOfMemory,
    InvalidState,
    UnsupportedQuery
}

#[derive(Debug)]
pub struct Error {
    pub kind: ErrorKind,
    pub context: String,
}

impl Error {
    pub fn new(kind: ErrorKind, context: String) -> Self {
        Error { kind, context }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.context)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error {
            kind: ErrorKind::Io,
            context: err.to_string(),
        }
    }
}

impl From<bincode::Error> for Error {
    fn from(err: bincode::Error) -> Self {
        Error {
            kind: ErrorKind::Parse,
            context: err.to_string(),
        }
    }
}

impl From<fst::Error> for Error {
    fn from(err: fst::Error) -> Self {
        Error {
            kind: ErrorKind::Internal,
            context: format!("FST error: {}", err),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;