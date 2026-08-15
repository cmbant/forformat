use std::fmt;

#[derive(Debug)]
pub enum FormatError {
    InputTooLarge,
    InvalidOption(String),
    Unsupported(String),
    Write(std::io::Error),
}

impl FormatError {
    pub fn is_broken_pipe(&self) -> bool {
        matches!(self, Self::Write(error) if error.kind() == std::io::ErrorKind::BrokenPipe)
    }
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputTooLarge => write!(f, "input exceeds the 4 GiB limit"),
            Self::InvalidOption(s) => write!(f, "invalid option: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::Write(e) => write!(f, "write error: {e}"),
        }
    }
}

impl std::error::Error for FormatError {}
