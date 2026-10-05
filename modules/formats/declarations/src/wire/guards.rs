//! Bound allocations before deserializing either carrier into owned values.

use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};

use crate::{
    DecodeError,
    codec::binary,
    validation::{MAX_CONTAINER, MAX_DEPTH, MAX_STRING, MAX_VALUES},
};

pub(crate) fn guard_messagepack(input: &[u8]) -> Result<(), DecodeError> {
    let mut scanner = Scanner {
        input,
        cursor: 0,
        values: 0,
    };
    scanner.value(0)?;
    if scanner.cursor != input.len() {
        return Err(binary("trailing MessagePack values"));
    }
    Ok(())
}

struct Scanner<'a> {
    input: &'a [u8],
    cursor: usize,
    values: usize,
}

impl Scanner<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], DecodeError> {
        let end = self
            .cursor
            .checked_add(count)
            .ok_or_else(|| binary("length overflow"))?;
        let bytes = self
            .input
            .get(self.cursor..end)
            .ok_or_else(|| binary("truncated MessagePack value"))?;
        self.cursor = end;
        Ok(bytes)
    }

    fn number(&mut self, bytes: usize) -> Result<usize, DecodeError> {
        Ok(self
            .take(bytes)?
            .iter()
            .fold(0usize, |value, byte| (value << 8) | usize::from(*byte)))
    }

    fn value(&mut self, depth: usize) -> Result<(), DecodeError> {
        self.values += 1;
        if self.values > MAX_VALUES || depth > MAX_DEPTH {
            return Err(binary("MessagePack capacity limit exceeded"));
        }
        let tag = self.take(1)?[0];
        match tag {
            0x00..=0x7F | 0xC0 | 0xC2 | 0xC3 | 0xE0..=0xFF => {},
            0xCC | 0xD0 => {
                self.take(1)?;
            },
            0xCD | 0xD1 => {
                self.take(2)?;
            },
            0xCE | 0xD2 => {
                self.take(4)?;
            },
            0xCF | 0xD3 => {
                self.take(8)?;
            },
            0xA0..=0xBF => self.string(usize::from(tag & 31))?,
            0xD9 => {
                let len = self.number(1)?;
                self.string(len)?;
            },
            0xDA => {
                let len = self.number(2)?;
                self.string(len)?;
            },
            0xDB => {
                let len = self.number(4)?;
                self.string(len)?;
            },
            0x90..=0x9F => self.array(usize::from(tag & 15), depth)?,
            0xDC => {
                let len = self.number(2)?;
                self.array(len, depth)?;
            },
            0xDD => {
                let len = self.number(4)?;
                self.array(len, depth)?;
            },
            _ => {
                return Err(binary(
                    "unexpected MessagePack tag for declaration wire format",
                ));
            },
        }
        Ok(())
    }

    fn string(&mut self, len: usize) -> Result<(), DecodeError> {
        if len > MAX_STRING {
            return Err(binary("MessagePack string exceeds capacity limit"));
        }
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes).map_err(binary)?;
        Ok(())
    }

    fn array(&mut self, len: usize, depth: usize) -> Result<(), DecodeError> {
        if len > MAX_CONTAINER || len > MAX_VALUES - self.values {
            return Err(binary("MessagePack container exceeds capacity limit"));
        }
        for _ in 0..len {
            self.value(depth + 1)?;
        }
        Ok(())
    }
}

pub(crate) fn guard_json(input: &[u8]) -> Result<(), DecodeError> {
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    let mut values = 0;
    Guard {
        values: &mut values,
        depth: 0,
    }
    .deserialize(&mut deserializer)
    .map_err(DecodeError::Syntax)?;
    deserializer.end().map_err(DecodeError::Syntax)
}

struct Guard<'a> {
    values: &'a mut usize,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for Guard<'_> {
    type Value = ();

    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        *self.values += 1;
        if *self.values > MAX_VALUES || self.depth > MAX_DEPTH {
            return Err(serde::de::Error::custom("JSON capacity limit exceeded"));
        }
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Guard<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("bounded JSON declaration data")
    }

    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> { Ok(()) }

    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> { Ok(()) }

    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> { Ok(()) }

    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> { Ok(()) }

    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> { Ok(()) }

    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<(), E> {
        if value.len() > MAX_STRING {
            Err(E::custom("JSON string exceeds capacity limit"))
        } else {
            Ok(())
        }
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<(), A::Error> {
        let mut count = 0;
        while sequence
            .next_element_seed(Guard {
                values: self.values,
                depth: self.depth + 1,
            })?
            .is_some()
        {
            count += 1;
            if count > MAX_CONTAINER {
                return Err(serde::de::Error::custom(
                    "JSON container exceeds capacity limit",
                ));
            }
        }
        Ok(())
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let mut count = 0;
        while map
            .next_key_seed(Guard {
                values: self.values,
                depth: self.depth + 1,
            })?
            .is_some()
        {
            count += 1;
            if count > MAX_CONTAINER {
                return Err(serde::de::Error::custom(
                    "JSON container exceeds capacity limit",
                ));
            }
            map.next_value_seed(Guard {
                values: self.values,
                depth: self.depth + 1,
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
