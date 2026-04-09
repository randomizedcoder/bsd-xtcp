use super::{OutputError, OutputSink};
use crate::proto_gen::tcpstats_reader::BatchMessage;
use std::io::{BufWriter, Write};

/// JSON Lines output sink. Writes one JSON object per line.
pub struct JsonSink<W: Write> {
    writer: BufWriter<W>,
    pretty: bool,
}

impl<W: Write> JsonSink<W> {
    pub fn new(writer: W, pretty: bool) -> Self {
        Self {
            writer: BufWriter::new(writer),
            pretty,
        }
    }
}

impl<W: Write> OutputSink for JsonSink<W> {
    fn emit(&mut self, batch: &BatchMessage) -> Result<(), OutputError> {
        if self.pretty {
            serde_json::to_writer_pretty(&mut self.writer, batch)?;
        } else {
            serde_json::to_writer(&mut self.writer, batch)?;
        }
        self.writer.write_all(b"\n")?;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), OutputError> {
        self.writer.flush()?;
        Ok(())
    }

    fn format_name(&self) -> &'static str {
        "jsonl"
    }
}
