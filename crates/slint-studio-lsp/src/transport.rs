//! LSP base protocol framing: `Content-Length` headers followed by a JSON body.

use std::io::{self, BufRead, Write};

/// Refuse absurdly large messages instead of allocating them.
const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

/// Reads one message body. Returns `Ok(None)` on a clean end of stream.
pub fn read_message(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut content_length = None;
    let mut saw_header = false;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return if saw_header {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "flusso terminato durante le intestazioni",
                ))
            } else {
                Ok(None)
            };
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            if saw_header {
                break;
            }
            continue; // tolerate stray blank lines between messages
        }
        saw_header = true;
        // Other headers (Content-Type) are ignored.
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            let length = value
                .trim()
                .parse::<usize>()
                .map_err(|_| invalid("Content-Length non valido"))?;
            content_length = Some(length);
        }
    }

    let length = content_length.ok_or_else(|| invalid("intestazione Content-Length mancante"))?;
    if length > MAX_MESSAGE_SIZE {
        return Err(invalid("messaggio troppo grande"));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    String::from_utf8(body)
        .map(Some)
        .map_err(|_| invalid("corpo del messaggio non UTF-8"))
}

/// Writes one message and flushes it.
pub fn write_message(writer: &mut impl Write, body: &str) -> io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body.as_bytes())?;
    writer.flush()
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn round_trip_multiple_messages() {
        let mut buffer = Vec::new();
        write_message(&mut buffer, r#"{"a":1}"#).unwrap();
        write_message(&mut buffer, r#"{"b":"è"}"#).unwrap();

        let mut reader = Cursor::new(buffer);
        assert_eq!(
            read_message(&mut reader).unwrap().as_deref(),
            Some(r#"{"a":1}"#)
        );
        assert_eq!(
            read_message(&mut reader).unwrap().as_deref(),
            Some(r#"{"b":"è"}"#)
        );
        assert_eq!(read_message(&mut reader).unwrap(), None);
    }

    #[test]
    fn length_counts_bytes_not_chars() {
        let mut buffer = Vec::new();
        write_message(&mut buffer, "\"è\"").unwrap();
        assert!(String::from_utf8_lossy(&buffer).starts_with("Content-Length: 4\r\n"));
    }

    #[test]
    fn extra_headers_and_case_are_accepted() {
        let raw = "content-length: 2\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{}";
        let mut reader = Cursor::new(raw.as_bytes());
        assert_eq!(read_message(&mut reader).unwrap().as_deref(), Some("{}"));
    }

    #[test]
    fn malformed_input_is_an_error() {
        let missing = "Content-Type: x\r\n\r\n{}";
        assert!(read_message(&mut Cursor::new(missing.as_bytes())).is_err());

        let truncated = "Content-Length: 10\r\n\r\n{}";
        assert!(read_message(&mut Cursor::new(truncated.as_bytes())).is_err());

        let eof_in_headers = "Content-Length: 2\r\n";
        assert!(read_message(&mut Cursor::new(eof_in_headers.as_bytes())).is_err());
    }
}
