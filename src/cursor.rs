use anyhow::{Context, Result, bail};

pub(crate) struct Cursor<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    pub(crate) fn read_var_i32(&mut self) -> Result<i32> {
        let mut value = 0i32;
        let mut shift = 0;

        loop {
            let byte = self.read_u8()?;
            // VarInt uses seven payload bits per byte and the high bit as a continuation flag.
            value |= ((byte & 0x7f) as i32) << shift;

            if byte & 0x80 == 0 {
                return Ok(value);
            }

            shift += 7;
            if shift >= 35 {
                bail!("VarInt is too large");
            }
        }
    }

    pub(crate) fn read_string(&mut self) -> Result<String> {
        let length = self.read_var_i32()?;
        if length < 0 {
            bail!("negative string length {length}");
        }

        let bytes = self.read_bytes(length as usize)?;
        String::from_utf8(bytes.to_vec()).context("invalid UTF-8 string")
    }

    pub(crate) fn read_u16(&mut self) -> Result<u16> {
        let bytes = self.read_bytes(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub(crate) fn read_i16(&mut self) -> Result<i16> {
        let bytes = self.read_bytes(2)?;
        Ok(i16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub(crate) fn read_i64(&mut self) -> Result<i64> {
        let bytes = self.read_bytes(8)?;
        Ok(i64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    pub(crate) fn read_f32(&mut self) -> Result<f32> {
        let bytes = self.read_bytes(4)?;
        Ok(f32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(crate) fn read_f64(&mut self) -> Result<f64> {
        let bytes = self.read_bytes(8)?;
        Ok(f64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    pub(crate) fn read_uuid(&mut self) -> Result<[u8; 16]> {
        let bytes = self.read_bytes(16)?;
        Ok(bytes.try_into().expect("slice length is 16"))
    }

    pub(crate) fn read_u8(&mut self) -> Result<u8> {
        let Some(byte) = self.data.get(self.position).copied() else {
            bail!("unexpected end of packet");
        };
        self.position += 1;
        Ok(byte)
    }

    pub(crate) fn read_bool(&mut self) -> Result<bool> {
        Ok(self.read_u8()? != 0)
    }

    pub(crate) fn read_block_pos(&mut self) -> Result<(i32, i32, i32)> {
        let value = self.read_i64()? as u64;
        // Packed block positions use x:26 bits, z:26 bits, y:12 bits.
        let mut x = (value >> 38) as i32;
        let mut y = (value & 0xfff) as i32;
        let mut z = ((value >> 12) & 0x3ff_ffff) as i32;
        if x >= 1 << 25 {
            x -= 1 << 26;
        }
        if y >= 1 << 11 {
            y -= 1 << 12;
        }
        if z >= 1 << 25 {
            z -= 1 << 26;
        }
        Ok((x, y, z))
    }

    pub(crate) fn remaining(&self) -> &'a [u8] {
        &self.data[self.position..]
    }

    fn read_bytes(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .context("packet cursor overflow")?;
        let Some(bytes) = self.data.get(self.position..end) else {
            bail!("unexpected end of packet");
        };
        self.position = end;
        Ok(bytes)
    }
}
