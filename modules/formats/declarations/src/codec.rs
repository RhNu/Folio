use std::io::Write;

use crate::{
    DeclarationBundle, SCHEMA_VERSION, validate,
    validation::{MAX_BYTES, invalid},
    wire,
};

// All header integers are little endian; payload is fixed-array MessagePack.
pub(crate) const MAGIC: &[u8; 8] = b"FOLDECL\0";
pub(crate) const HEADER_SIZE: usize = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclarationFormat {
    Json,
    Binary,
}

#[derive(Debug)]
pub enum DecodeError {
    Syntax(serde_json::Error),
    Binary(String),
    UnsupportedSchema(u32),
    Invalid { field: String, reason: &'static str },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Syntax(error) => write!(f, "invalid declaration JSON: {error}"),
            Self::Binary(error) => write!(f, "invalid binary declaration: {error}"),
            Self::UnsupportedSchema(version) => {
                write!(f, "unsupported declaration schema {version}")
            }
            Self::Invalid { field, reason } => write!(f, "invalid {field}: {reason}"),
        }
    }
}
impl std::error::Error for DecodeError {}

/// Detect the carrier from bytes and apply the same declaration validator.
pub fn decode(input: &[u8]) -> Result<DeclarationBundle, DecodeError> {
    if input.len() > MAX_BYTES {
        return Err(invalid("input", "carrier exceeds capacity limit"));
    }
    let bundle = if input.starts_with(MAGIC) {
        decode_binary(input)?
    } else {
        wire::guard_json(input)?;
        serde_json::from_slice(input).map_err(DecodeError::Syntax)?
    };
    validate(&bundle)?;
    tracing::debug!(
        scripts = bundle.scripts.len(),
        binary = input.starts_with(MAGIC),
        "decoded declarations"
    );
    Ok(bundle)
}

/// Encode an already validated declaration model in either supported carrier.
pub fn encode(
    bundle: &DeclarationBundle,
    format: DeclarationFormat,
) -> Result<Vec<u8>, DecodeError> {
    validate(bundle)?;
    let bytes = match format {
        DeclarationFormat::Json => {
            serde_json::to_vec_pretty(bundle).map_err(DecodeError::Syntax)?
        }
        DeclarationFormat::Binary => encode_binary(bundle)?,
    };
    if bytes.len() > MAX_BYTES {
        return Err(invalid("output", "carrier exceeds capacity limit"));
    }
    if format == DeclarationFormat::Json {
        wire::guard_json(&bytes)?;
    }
    tracing::debug!(
        scripts = bundle.scripts.len(),
        bytes = bytes.len(),
        ?format,
        "encoded declarations"
    );
    Ok(bytes)
}

fn encode_binary(bundle: &DeclarationBundle) -> Result<Vec<u8>, DecodeError> {
    let payload = wire::encode(bundle)?;
    if payload.len() > MAX_BYTES {
        return Err(invalid("payload", "payload exceeds capacity limit"));
    }
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(6));
    encoder.write_all(&payload).map_err(binary)?;
    let compressed = encoder.finish().map_err(binary)?;
    let mut result = Vec::with_capacity(HEADER_SIZE + compressed.len());
    result.extend_from_slice(MAGIC);
    result.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    result.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    result.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
    result.extend_from_slice(blake3::hash(&payload).as_bytes());
    result.extend_from_slice(&compressed);
    Ok(result)
}

fn decode_binary(input: &[u8]) -> Result<DeclarationBundle, DecodeError> {
    if input.len() < HEADER_SIZE {
        return Err(binary("truncated header"));
    }
    let schema = u32::from_le_bytes(input[8..12].try_into().expect("header checked"));
    if schema != SCHEMA_VERSION {
        return Err(DecodeError::UnsupportedSchema(schema));
    }
    let raw_len = u64::from_le_bytes(input[12..20].try_into().expect("header checked"));
    let compressed_len = u64::from_le_bytes(input[20..28].try_into().expect("header checked"));
    if raw_len > MAX_BYTES as u64 || compressed_len > MAX_BYTES as u64 {
        return Err(binary("length exceeds capacity limit"));
    }
    if compressed_len as usize != input.len() - HEADER_SIZE {
        return Err(binary("compressed length mismatch or trailing bytes"));
    }
    let mut decoder = flate2::Decompress::new(false);
    let mut payload = Vec::with_capacity(raw_len as usize + 1);
    let status = decoder
        .decompress_vec(
            &input[HEADER_SIZE..],
            &mut payload,
            flate2::FlushDecompress::Finish,
        )
        .map_err(binary)?;
    if status != flate2::Status::StreamEnd {
        return Err(binary(
            "incomplete DEFLATE stream or declared length exceeded",
        ));
    }
    if payload.len() as u64 != raw_len {
        return Err(binary("decompressed length mismatch"));
    }
    if decoder.total_in() != compressed_len {
        return Err(binary("trailing compressed payload"));
    }
    if blake3::hash(&payload).as_bytes() != &input[28..60] {
        return Err(binary("payload digest mismatch"));
    }
    let bundle = wire::decode(&payload)?;
    if bundle.schema != schema {
        return Err(binary("payload/header schema mismatch"));
    }
    Ok(bundle)
}

pub(crate) fn binary(error: impl std::fmt::Display) -> DecodeError {
    DecodeError::Binary(error.to_string())
}

#[cfg(test)]
mod tests;
