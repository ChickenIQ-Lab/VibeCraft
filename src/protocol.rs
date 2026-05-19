use anyhow::{Result, bail};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAX_PACKET_LENGTH: i32 = 2_097_152;

pub(crate) async fn read_packet<R: AsyncRead + Unpin>(stream: &mut R) -> Result<Vec<u8>> {
    // Every Minecraft packet starts with a VarInt byte length.
    let length = read_var_i32_async(stream).await?;
    if !(0..=MAX_PACKET_LENGTH).contains(&length) {
        bail!("invalid packet length {length}");
    }
    let mut data = vec![0; length as usize];
    stream.read_exact(&mut data).await?;
    Ok(data)
}

pub(crate) async fn write_packet<W: AsyncWrite + Unpin>(
    stream: &mut W,
    payload: &[u8],
) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, payload.len() as i32);
    packet.extend_from_slice(payload);
    stream.write_all(&packet).await?;
    Ok(())
}

async fn read_var_i32_async<R: AsyncRead + Unpin>(stream: &mut R) -> Result<i32> {
    let mut value = 0i32;
    let mut position = 0;
    loop {
        let byte = stream.read_u8().await?;
        // VarInt uses seven payload bits per byte and the high bit as a continuation flag.
        value |= ((byte & 0x7f) as i32) << position;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        position += 7;
        if position >= 35 {
            bail!("VarInt is too large");
        }
    }
}

pub(crate) fn write_var_i32(out: &mut Vec<u8>, mut value: i32) {
    loop {
        if value & !0x7f == 0 {
            out.push(value as u8);
            return;
        }
        // Logical shift keeps negative protocol values encoded in the Minecraft format.
        out.push(((value & 0x7f) | 0x80) as u8);
        value = ((value as u32) >> 7) as i32;
    }
}

pub(crate) fn write_string(out: &mut Vec<u8>, value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    if bytes.len() > i16::MAX as usize {
        bail!("string is too long");
    }
    write_var_i32(out, bytes.len() as i32);
    out.extend_from_slice(bytes);
    Ok(())
}

pub(crate) fn pack_position(x: i32, y: i32, z: i32) -> i64 {
    // Packed block positions use x:26 bits, z:26 bits, y:12 bits.
    (((x as i64) & 0x3ff_ffff) << 38) | (((z as i64) & 0x3ff_ffff) << 12) | ((y as i64) & 0xfff)
}
