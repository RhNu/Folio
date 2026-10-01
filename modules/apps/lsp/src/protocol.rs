//! LSP message framing, position encodings, and local file URIs.
use super::LspError;
use folio_ide::{Position, PositionEncoding, Range};
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub(super) type Output = Arc<Mutex<io::Stdout>>;

pub(super) fn send(output: &Output, value: &Value) -> Result<(), LspError> {
    let bytes = serde_json::to_vec(value).map_err(|error| LspError::Protocol(error.to_string()))?;
    let mut stream = output
        .lock()
        .map_err(|_| LspError::Protocol("stdout lock poisoned".into()))?;
    write!(stream, "Content-Length: {}\r\n\r\n", bytes.len())?;
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}

pub(super) fn read_message(reader: &mut impl BufRead) -> Result<Option<Value>, LspError> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return if length.is_none() {
                Ok(None)
            } else {
                Err(LspError::Protocol("incomplete LSP header".into()))
            };
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':')
            && key.eq_ignore_ascii_case("Content-Length")
        {
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| LspError::Protocol("invalid Content-Length".into()))?,
            );
        }
    }
    let length = length.ok_or_else(|| LspError::Protocol("missing Content-Length".into()))?;
    if length > 16 * 1024 * 1024 {
        return Err(LspError::Protocol("LSP message exceeds 16 MiB".into()));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| LspError::Protocol(error.to_string()))
}

pub(super) fn negotiate_encoding(params: &Value) -> PositionEncoding {
    let encodings = params["capabilities"]["general"]["positionEncodings"].as_array();
    if encodings.is_some_and(|items| items.iter().any(|item| item.as_str() == Some("utf-8"))) {
        PositionEncoding::Utf8
    } else {
        PositionEncoding::Utf16
    }
}

pub(super) fn parse_position(value: &Value) -> Option<Position> {
    Some(Position {
        line: u32::try_from(value["line"].as_u64()?).ok()?,
        character: u32::try_from(value["character"].as_u64()?).ok()?,
    })
}

pub(super) fn parse_range(value: &Value) -> Option<Range> {
    Some(Range {
        start: parse_position(&value["start"])?,
        end: parse_position(&value["end"])?,
    })
}

pub(super) fn range_json(range: Range) -> Value {
    json!({"start":{"line":range.start.line,"character":range.start.character},"end":{"line":range.end.line,"character":range.end.character}})
}

/// Converts local file URIs without treating percent escapes as source text.
pub(super) fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let encoded = uri.strip_prefix("file://")?;
    let decoded = percent_decode(encoded)?;
    #[cfg(windows)]
    {
        let path = decoded
            .strip_prefix('/')
            .unwrap_or(&decoded)
            .replace('/', "\\");
        canonical_or_parent(Path::new(&path))
    }
    #[cfg(not(windows))]
    {
        canonical_or_parent(Path::new(&decoded))
    }
}

fn canonical_or_parent(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok().or_else(|| {
        let parent = std::fs::canonicalize(path.parent()?).ok()?;
        Some(parent.join(path.file_name()?))
    })
}

pub(super) fn path_to_uri(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    let path = path.strip_prefix("//?/").unwrap_or(&path).to_owned();
    let prefix = if path.starts_with('/') {
        "file://"
    } else {
        "file:///"
    };
    let mut result = prefix.to_owned();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.' | b'~') {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}

pub(super) fn percent_decode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let source = text.as_bytes();
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'%' {
            let hex = source.get(index + 1..index + 3)?;
            bytes.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            index += 3;
        } else {
            bytes.push(source[index]);
            index += 1;
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests;
