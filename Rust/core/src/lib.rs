//! Platform-independent ONScripter runtime and game-data readers.

pub mod archive;
pub mod assets;
pub mod scene;
pub mod script;
pub mod text;
pub mod vm;

use std::{fmt, io};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Invalid(String),
    Limit(&'static str),
    Script { line: u32, message: String },
}

impl Error {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Invalid(message) => f.write_str(message),
            Self::Limit(resource) => write!(f, "{resource} exceeds its resource budget"),
            Self::Script { line, message } => write!(f, "line {line}: {message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub script_bytes: usize,
    pub asset_bytes: usize,
    pub index_bytes: usize,
    pub filename_bytes: usize,
    pub program_bytes: usize,
    pub state_bytes: usize,
    pub call_depth: usize,
    pub expression_depth: usize,
    pub instructions_per_tick: usize,
    pub image_bytes: usize,
    pub texture_bytes: usize,
    pub image_dimension: u32,
    pub sprites: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // Match the existing compressed-script format's 256 MiB ceiling.
            script_bytes: 0x1000_0000,
            asset_bytes: 0x1000_0000,
            index_bytes: 128 * 1024 * 1024,
            filename_bytes: 4095,
            program_bytes: 128 * 1024 * 1024,
            state_bytes: 16 * 1024 * 1024,
            call_depth: 1024,
            expression_depth: 128,
            instructions_per_tick: 10_000,
            image_bytes: 128 * 1024 * 1024,
            texture_bytes: 256 * 1024 * 1024,
            image_dimension: 16_384,
            sprites: 1000,
        }
    }
}

pub(crate) fn read_limited(reader: &mut impl io::Read, maximum: usize) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut chunk = [0; 16 * 1024];
    loop {
        let count = match reader.read(&mut chunk) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            return Ok(output);
        }
        if count > maximum.saturating_sub(output.len()) {
            return Err(Error::Limit("input size"));
        }
        output
            .try_reserve(count)
            .map_err(|_| Error::Limit("input allocation"))?;
        output.extend_from_slice(&chunk[..count]);
    }
}
