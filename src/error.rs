use thiserror::Error;

#[derive(Debug, Error)]
pub enum HiveError {
    #[error("{0}")]
    Message(String),
}

impl From<String> for HiveError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}
