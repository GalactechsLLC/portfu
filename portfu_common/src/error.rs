use std::fmt::{Debug, Display, Formatter};

#[derive(Debug)]
pub enum PortfuError {
    Parsing(String),
    Internal(String),
    Io(std::io::Error),
}

impl Display for PortfuError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PortfuError::Parsing(msg) => write!(f, "{}", msg),
            PortfuError::Internal(msg) => write!(f, "{}", msg),
            PortfuError::Io(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for PortfuError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PortfuError::Parsing(_) => None,
            PortfuError::Internal(_) => None,
            PortfuError::Io(e) => Some(e),
        }
    }
}
