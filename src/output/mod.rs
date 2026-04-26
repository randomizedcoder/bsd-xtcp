pub mod json;

use crate::proto_gen::tcpstats_reader::BatchMessage;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OutputError {
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Trait for output sinks that emit BatchMessage data.
pub trait OutputSink {
    fn emit(&mut self, batch: &BatchMessage) -> Result<(), OutputError>;
    fn flush(&mut self) -> Result<(), OutputError>;
    fn format_name(&self) -> &'static str;
}
