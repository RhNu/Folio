use super::*;

pub(crate) struct BinaryWriter {
    pub(crate) endianness: Endianness,
    pub(crate) bytes: Vec<u8>,
}

impl BinaryWriter {
    pub(crate) fn new(endianness: Endianness) -> Self {
        Self {
            endianness,
            bytes: Vec::new(),
        }
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub(crate) fn write_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub(crate) fn write_u16(&mut self, value: u16) {
        match self.endianness {
            Endianness::Little => self.bytes.extend(value.to_le_bytes()),
            Endianness::Big => self.bytes.extend(value.to_be_bytes()),
        }
    }

    pub(crate) fn write_u32(&mut self, value: u32) {
        match self.endianness {
            Endianness::Little => self.bytes.extend(value.to_le_bytes()),
            Endianness::Big => self.bytes.extend(value.to_be_bytes()),
        }
    }

    pub(crate) fn write_i32(&mut self, value: i32) {
        match self.endianness {
            Endianness::Little => self.bytes.extend(value.to_le_bytes()),
            Endianness::Big => self.bytes.extend(value.to_be_bytes()),
        }
    }

    pub(crate) fn write_u64(&mut self, value: u64) {
        match self.endianness {
            Endianness::Little => self.bytes.extend(value.to_le_bytes()),
            Endianness::Big => self.bytes.extend(value.to_be_bytes()),
        }
    }

    pub(crate) fn write_f32(&mut self, value: f32) {
        match self.endianness {
            Endianness::Little => self.bytes.extend(value.to_le_bytes()),
            Endianness::Big => self.bytes.extend(value.to_be_bytes()),
        }
    }

    pub(crate) fn write_len_u16(
        &mut self,
        what: &'static str,
        len: usize,
    ) -> Result<(), PexWriteError> {
        ensure_u16(what, len)?;
        self.write_u16(len as u16);
        Ok(())
    }

    pub(crate) fn write_counted_str(
        &mut self,
        what: &'static str,
        value: &str,
    ) -> Result<(), PexWriteError> {
        if value.len() > u16::MAX as usize {
            return Err(PexWriteError::StringTooLong {
                what,
                len: value.len(),
            });
        }

        self.write_u16(value.len() as u16);
        self.bytes.extend(value.as_bytes());
        Ok(())
    }

    pub(crate) fn write_string_id(
        &mut self,
        id: PexStringId,
        table_len: usize,
    ) -> Result<(), PexWriteError> {
        if usize::from(id.index()) >= table_len {
            return Err(PexWriteError::StringIdOutOfRange { id, table_len });
        }

        self.write_u16(id.index());
        Ok(())
    }

    pub(crate) fn write_user_flags(&mut self, user_flags: u32) {
        self.write_u32(user_flags);
    }
}

pub(crate) struct BinaryReader<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) offset: usize,
    pub(crate) endianness: Endianness,
}

impl<'a> BinaryReader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            offset: 0,
            endianness: Endianness::Big,
        }
    }

    pub(crate) const fn offset(&self) -> usize {
        self.offset
    }

    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    pub(crate) fn read_magic(&mut self) -> Result<u32, PexReadError> {
        let bytes = self.take(4, "magic")?;
        if bytes == PEX_MAGIC.to_be_bytes() {
            self.endianness = Endianness::Big;
            Ok(PEX_MAGIC)
        } else if bytes == PEX_MAGIC.to_le_bytes() {
            self.endianness = Endianness::Little;
            Ok(PEX_MAGIC)
        } else {
            Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        }
    }

    pub(crate) fn read_u8(&mut self, what: &'static str) -> Result<u8, PexReadError> {
        Ok(self.take(1, what)?[0])
    }

    pub(crate) fn read_u16(&mut self, what: &'static str) -> Result<u16, PexReadError> {
        let bytes = self.take(2, what)?;
        Ok(match self.endianness {
            Endianness::Little => u16::from_le_bytes([bytes[0], bytes[1]]),
            Endianness::Big => u16::from_be_bytes([bytes[0], bytes[1]]),
        })
    }

    pub(crate) fn read_i32(&mut self, what: &'static str) -> Result<i32, PexReadError> {
        let bytes = self.take(4, what)?;
        Ok(match self.endianness {
            Endianness::Little => i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            Endianness::Big => i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        })
    }

    pub(crate) fn read_u32(&mut self, what: &'static str) -> Result<u32, PexReadError> {
        let bytes = self.take(4, what)?;
        Ok(match self.endianness {
            Endianness::Little => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            Endianness::Big => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        })
    }

    pub(crate) fn read_u64(&mut self, what: &'static str) -> Result<u64, PexReadError> {
        let bytes = self.take(8, what)?;
        Ok(match self.endianness {
            Endianness::Little => u64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]),
            Endianness::Big => u64::from_be_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]),
        })
    }

    pub(crate) fn read_f32(&mut self, what: &'static str) -> Result<f32, PexReadError> {
        let bytes = self.take(4, what)?;
        Ok(match self.endianness {
            Endianness::Little => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            Endianness::Big => f32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        })
    }

    pub(crate) fn read_counted_str(&mut self, what: &'static str) -> Result<String, PexReadError> {
        let len = usize::from(self.read_u16(what)?);
        let offset = self.offset;
        let bytes = self.take(len, what)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| PexReadError::InvalidUtf8 { offset, what })
    }

    pub(crate) fn read_string_id(
        &mut self,
        table_len: usize,
        what: &'static str,
    ) -> Result<PexStringId, PexReadError> {
        let id = PexStringId::new(self.read_u16(what)?);
        if usize::from(id.index()) >= table_len {
            return Err(PexReadError::StringIdOutOfRange {
                id,
                table_len,
                what,
            });
        }
        Ok(id)
    }

    pub(crate) fn take(
        &mut self,
        len: usize,
        what: &'static str,
    ) -> Result<&'a [u8], PexReadError> {
        let offset = self.offset;
        let remaining = self.remaining();
        if remaining < len {
            return Err(PexReadError::Truncated {
                offset,
                needed: len,
                remaining,
                what,
            });
        }
        self.offset += len;
        Ok(&self.bytes[offset..offset + len])
    }
}
