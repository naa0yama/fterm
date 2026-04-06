//! ANSI escape stripping and timestamp prepending for log streams.
//!
//! Replaces the Fish implementation's `ansifilter | awk '{print strftime(...)}'`
//! pipeline with a pure Rust solution used via `fterm log-filter`.

use std::io::{BufRead, BufReader, BufWriter, Read, Write};

use anyhow::{Context, Result};
use chrono::Local;

/// Process a byte stream: strip ANSI escapes and prepend timestamps.
///
/// Reads lines from `input`, strips ANSI escape sequences, prepends an
/// ISO 8601 timestamp, and writes to `output`. Each line is flushed
/// immediately (matching the original `awk fflush()` behavior).
///
/// # Errors
/// Returns an error if reading from input or writing to output fails.
pub fn process_stream<R: Read, W: Write>(input: R, output: W) -> Result<()> {
    let reader = BufReader::new(input);
    let mut writer = BufWriter::new(output);

    for line_result in reader.lines() {
        let line = line_result.context("failed to read line from input")?;
        let stripped = strip_ansi(&line);
        let timestamp = Local::now().format("%Y-%m-%dT%H:%M:%S%z");
        writeln!(writer, "[{timestamp}] {stripped}").context("failed to write to output")?;
        writer.flush().context("failed to flush output")?;
    }

    Ok(())
}

/// Strip ANSI escape sequences from a string.
fn strip_ansi(input: &str) -> String {
    let bytes = strip_ansi_escapes::strip(input);
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]

    use super::{process_stream, strip_ansi};

    #[test]
    fn strip_ansi_plain_text() {
        assert_eq!(strip_ansi("hello world"), "hello world");
    }

    #[test]
    fn strip_ansi_color_codes() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m text"), "red text");
    }

    #[test]
    fn strip_ansi_bold_and_reset() {
        assert_eq!(strip_ansi("\x1b[1mbold\x1b[0m"), "bold");
    }

    #[test]
    fn strip_ansi_cursor_movement() {
        assert_eq!(strip_ansi("\x1b[2Jhello"), "hello");
    }

    #[test]
    fn strip_ansi_empty_input() {
        assert_eq!(strip_ansi(""), "");
    }

    #[cfg(not(miri))]
    #[test]
    fn process_stream_single_line() {
        let input = b"hello\n";
        let mut output = Vec::new();

        process_stream(&input[..], &mut output).unwrap();

        let result = String::from_utf8(output).unwrap();
        // Verify timestamp format and content
        assert!(result.starts_with('['));
        assert!(result.contains("] hello\n"));
    }

    #[cfg(not(miri))]
    #[test]
    fn process_stream_multiple_lines() {
        let input = b"line1\nline2\nline3\n";
        let mut output = Vec::new();

        process_stream(&input[..], &mut output).unwrap();

        let result = String::from_utf8(output).unwrap();
        let lines: Vec<&str> = result.trim().lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("] line1"));
        assert!(lines[1].contains("] line2"));
        assert!(lines[2].contains("] line3"));
    }

    #[cfg(not(miri))]
    #[test]
    fn process_stream_strips_ansi() {
        let input = b"\x1b[31mred\x1b[0m text\n";
        let mut output = Vec::new();

        process_stream(&input[..], &mut output).unwrap();

        let result = String::from_utf8(output).unwrap();
        assert!(result.contains("] red text\n"));
        assert!(!result.contains("\x1b["));
    }

    #[cfg(not(miri))]
    #[test]
    fn process_stream_empty_input() {
        let input = b"";
        let mut output = Vec::new();

        process_stream(&input[..], &mut output).unwrap();

        assert!(output.is_empty());
    }
}
