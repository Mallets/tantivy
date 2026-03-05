use std::io;
use std::io::{Read, Write};

use super::BinarySerializable;

/// Variable int serializes a u128 number
pub fn serialize_vint_u128(mut val: u128, output: &mut Vec<u8>) {
    loop {
        let next_byte: u8 = (val % 128u128) as u8;
        val /= 128u128;
        if val == 0 {
            output.push(next_byte | STOP_BIT);
            return;
        } else {
            output.push(next_byte);
        }
    }
}

///   Wrapper over a `u128` that serializes as a variable int.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VIntU128(pub u128);

impl BinarySerializable for VIntU128 {
    fn serialize<W: Write + ?Sized>(&self, writer: &mut W) -> io::Result<()> {
        let mut buffer = vec![];
        serialize_vint_u128(self.0, &mut buffer);
        writer.write_all(&buffer)
    }

    #[allow(clippy::unbuffered_bytes)]
    fn deserialize<R: Read>(reader: &mut R) -> io::Result<Self> {
        #[allow(clippy::unbuffered_bytes)]
        let mut bytes = reader.bytes();
        let mut result = 0u128;
        let mut shift = 0u64;
        loop {
            match bytes.next() {
                Some(Ok(b)) => {
                    result |= u128::from(b % 128u8) << shift;
                    if b >= STOP_BIT {
                        return Ok(VIntU128(result));
                    }
                    shift += 7;
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Reach end of buffer while reading VInt",
                    ));
                }
            }
        }
    }
}

///   Wrapper over a `u64` that serializes as a variable int.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VInt(pub u64);

const STOP_BIT: u8 = 128;

#[inline]
pub fn serialize_vint_u32(val: u32, buf: &mut [u8; 8]) -> &[u8] {
    const START_2: u64 = 1 << 7;
    const START_3: u64 = 1 << 14;
    const START_4: u64 = 1 << 21;
    const START_5: u64 = 1 << 28;

    const MASK_1: u64 = 127;
    const MASK_2: u64 = MASK_1 << 7;
    const MASK_3: u64 = MASK_2 << 7;
    const MASK_4: u64 = MASK_3 << 7;
    const MASK_5: u64 = MASK_4 << 7;

    let val = u64::from(val);
    const STOP_BIT: u64 = 128u64;
    let (res, num_bytes) = if val < START_2 {
        (val | STOP_BIT, 1)
    } else if val < START_3 {
        (
            (val & MASK_1) | ((val & MASK_2) << 1) | (STOP_BIT << (8)),
            2,
        )
    } else if val < START_4 {
        (
            (val & MASK_1) | ((val & MASK_2) << 1) | ((val & MASK_3) << 2) | (STOP_BIT << (8 * 2)),
            3,
        )
    } else if val < START_5 {
        (
            (val & MASK_1)
                | ((val & MASK_2) << 1)
                | ((val & MASK_3) << 2)
                | ((val & MASK_4) << 3)
                | (STOP_BIT << (8 * 3)),
            4,
        )
    } else {
        (
            (val & MASK_1)
                | ((val & MASK_2) << 1)
                | ((val & MASK_3) << 2)
                | ((val & MASK_4) << 3)
                | ((val & MASK_5) << 4)
                | (STOP_BIT << (8 * 4)),
            5,
        )
    };
    *buf = res.to_le_bytes();
    &buf[0..num_bytes]
}

/// Returns the number of bytes covered by a
/// serialized vint `u32`.
///
/// Expects a buffer data that starts
/// by the serialized `vint`, scans at most 5 bytes ahead until
/// it finds the vint final byte.
///
/// # May Panic
/// If the payload does not start by a valid `vint`
fn vint_len(data: &[u8]) -> usize {
    for (i, &val) in data.iter().enumerate().take(5) {
        if val >= STOP_BIT {
            return i + 1;
        }
    }
    panic!("Corrupted data. Invalid VInt 32");
}

/// Reads a vint `u32` from a buffer, and
/// consumes its payload data.
///
/// # Panics
///
/// If the buffer does not start by a valid
/// vint payload
pub fn read_u32_vint(data: &mut &[u8]) -> u32 {
    let (result, vlen) = read_u32_vint_no_advance(data);
    *data = &data[vlen..];
    result
}

pub fn read_u32_vint_no_advance(data: &[u8]) -> (u32, usize) {
    let vlen = vint_len(data);
    let mut result = 0u32;
    let mut shift = 0u64;
    for &b in &data[..vlen] {
        result |= u32::from(b & 127u8) << shift;
        shift += 7;
    }
    (result, vlen)
}

pub fn read_u32_vint_short(data: &mut &[u8]) -> u32 {
    let (result, vlen) = decode_vint_u32_short(data);
    *data = &data[vlen..];
    result
}

/// Write a `u32` as a vint payload.
pub fn write_u32_vint<W: io::Write + ?Sized>(val: u32, writer: &mut W) -> io::Result<()> {
    let mut buf = [0u8; 8];
    let data = serialize_vint_u32(val, &mut buf);
    writer.write_all(data)
}

pub fn write_u32_vint_short<W: io::Write + ?Sized>(val: u32, writer: &mut W) -> io::Result<()> {
    let mut buf = [0u8; VLE_U32_SHORT_LEN_MAX];
    let data = serialize_vint_u32_short(val, &mut buf);
    writer.write_all(data)
}

// ---------------------------------------------------------------------------
// Short VLE u32 encoding
// ---------------------------------------------------------------------------

/// Buffer size for encoding a `u32` in the short VLE format.
/// The encoder writes into a `u64`, so the buffer is 8 bytes,
/// even though the actual encoded length is at most 5.
pub const VLE_U32_SHORT_LEN_MAX: usize = 8;

/// Maximum value encodable as a short VLE u32: `u32::MAX`.
pub const VLE_U32_SHORT_VAL_MAX: u32 = u32::MAX;

const SHORT_B1: u32 = 251;
const SHORT_B2: u32 = 252 + 0xFF;
const SHORT_B3: u32 = 252 + 0xFFFF;
const SHORT_B4: u32 = 252 + 0xFFFFFF;

/// Short VLE-encodes a `u32` into `buf`, returning the encoded slice.
///
/// First byte 0–251 stores the value directly. Bytes 252–255 are length tags
/// indicating 1–4 trailing bytes that hold `(val - 252)` in little-endian.
#[inline]
pub fn serialize_vint_u32_short(val: u32, buf: &mut [u8; VLE_U32_SHORT_LEN_MAX]) -> &[u8] {
    let n = if val <= SHORT_B1 {
        *buf = u64::from(val).to_le_bytes();
        1
    } else if val <= SHORT_B2 {
        let rem = u64::from(val - 252);
        *buf = (252u64 | (rem << 8)).to_le_bytes();
        2
    } else if val <= SHORT_B3 {
        let rem = u64::from(val - 252);
        *buf = (253u64 | (rem << 8)).to_le_bytes();
        3
    } else if val <= SHORT_B4 {
        let rem = u64::from(val - 252);
        *buf = (254u64 | (rem << 8)).to_le_bytes();
        4
    } else {
        let rem = u64::from(val - 252);
        *buf = (255u64 | (rem << 8)).to_le_bytes();
        5
    };
    &buf[..n]
}

/// Decodes a short VLE `u32` from `buf`.
/// Returns `(value, bytes_consumed)`.
///
/// # Panics
///
/// If `buf` is empty or too short for the encoded length.
pub fn decode_vint_u32_short(buf: &[u8]) -> (u32, usize) {
    assert!(!buf.is_empty(), "decode_vint_u32_short: empty buffer");
    let tag = buf[0];
    if tag <= 251 {
        (tag as u32, 1)
    } else {
        let extra = (tag - 251) as usize;
        assert!(buf.len() >= 1 + extra, "decode_vint_u32_short: buffer too short");
        let mut v = [0u8; 4];
        v[..extra].copy_from_slice(&buf[1..1 + extra]);
        (u32::from_le_bytes(v) + 252, 1 + extra)
    }
}

/// Returns the number of bytes needed to short VLE-encode `x`.
pub const fn vle_u32_short_len(x: u32) -> usize {
    if x <= SHORT_B1 {
        1
    } else if x <= SHORT_B2 {
        2
    } else if x <= SHORT_B3 {
        3
    } else if x <= SHORT_B4 {
        4
    } else {
        5
    }
}

impl VInt {
    pub fn val(&self) -> u64 {
        self.0
    }

    pub fn deserialize_u64<R: Read>(reader: &mut R) -> io::Result<u64> {
        VInt::deserialize(reader).map(|vint| vint.0)
    }

    pub fn serialize_into_vec(&self, output: &mut Vec<u8>) {
        let mut buffer = [0u8; 10];
        let num_bytes = self.serialize_into(&mut buffer);
        output.extend(&buffer[0..num_bytes]);
    }

    pub fn serialize_into(&self, buffer: &mut [u8; 10]) -> usize {
        let mut remaining = self.0;
        for (i, b) in buffer.iter_mut().enumerate() {
            let next_byte: u8 = (remaining % 128u64) as u8;
            remaining /= 128u64;
            if remaining == 0u64 {
                *b = next_byte | STOP_BIT;
                return i + 1;
            } else {
                *b = next_byte;
            }
        }
        unreachable!();
    }
}

impl BinarySerializable for VInt {
    fn serialize<W: Write + ?Sized>(&self, writer: &mut W) -> io::Result<()> {
        let mut buffer = [0u8; 10];
        let num_bytes = self.serialize_into(&mut buffer);
        writer.write_all(&buffer[0..num_bytes])
    }

    #[allow(clippy::unbuffered_bytes)]
    fn deserialize<R: Read>(reader: &mut R) -> io::Result<Self> {
        #[allow(clippy::unbuffered_bytes)]
        let mut bytes = reader.bytes();
        let mut result = 0u64;
        let mut shift = 0u64;
        loop {
            match bytes.next() {
                Some(Ok(b)) => {
                    result |= u64::from(b % 128u8) << shift;
                    if b >= STOP_BIT {
                        return Ok(VInt(result));
                    }
                    shift += 7;
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Reach end of buffer while reading VInt",
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use super::{
        BinarySerializable, SHORT_B1, SHORT_B2, SHORT_B3, SHORT_B4, VInt, VLE_U32_SHORT_LEN_MAX,
        VLE_U32_SHORT_VAL_MAX, decode_vint_u32_short, serialize_vint_u32, serialize_vint_u32_short,
        vle_u32_short_len,
    };

    fn aux_test_vint(val: u64) {
        let mut v = [14u8; 10];
        let num_bytes = VInt(val).serialize_into(&mut v);
        for el in &v[num_bytes..10] {
            assert_eq!(el, &14u8);
        }
        assert!(num_bytes > 0);
        if num_bytes < 10 {
            assert!(1u64 << (7 * num_bytes) > val);
        }
        if num_bytes > 1 {
            assert!(1u64 << (7 * (num_bytes - 1)) <= val);
        }
        let serdeser_val = VInt::deserialize(&mut &v[..]).unwrap();
        assert_eq!(val, serdeser_val.0);
    }

    #[test]
    fn test_vint() {
        aux_test_vint(0);
        aux_test_vint(1);
        aux_test_vint(5);
        aux_test_vint(u64::MAX);
        for i in 1..9 {
            let power_of_128 = 1u64 << (7 * i);
            aux_test_vint(power_of_128 - 1u64);
            aux_test_vint(power_of_128);
            aux_test_vint(power_of_128 + 1u64);
        }
        aux_test_vint(10);
    }

    fn aux_test_serialize_vint_u32(val: u32) {
        let mut buffer = [0u8; 10];
        let mut buffer2 = [0u8; 8];
        let len_vint = VInt(val as u64).serialize_into(&mut buffer);
        let res2 = serialize_vint_u32(val, &mut buffer2);
        assert_eq!(&buffer[..len_vint], res2, "array wrong for {val}");
    }

    #[test]
    fn test_vint_u32() {
        aux_test_serialize_vint_u32(0);
        aux_test_serialize_vint_u32(1);
        aux_test_serialize_vint_u32(5);
        for i in 1..3 {
            let power_of_128 = 1u32 << (7 * i);
            aux_test_serialize_vint_u32(power_of_128 - 1u32);
            aux_test_serialize_vint_u32(power_of_128);
            aux_test_serialize_vint_u32(power_of_128 + 1u32);
        }
        aux_test_serialize_vint_u32(u32::MAX);
    }

    // ---- Short VLE u32 tests ----

    fn aux_test_short(val: u32) {
        let mut buf = [0u8; VLE_U32_SHORT_LEN_MAX];
        let encoded = serialize_vint_u32_short(val, &mut buf);
        let num_bytes = encoded.len();

        assert!(num_bytes > 0 && num_bytes <= 5);
        assert_eq!(num_bytes, vle_u32_short_len(val));

        let (decoded, consumed) = decode_vint_u32_short(&buf[..num_bytes]);
        assert_eq!(decoded, val, "roundtrip failed for {val}");
        assert_eq!(consumed, num_bytes);
    }

    #[test]
    fn test_short_roundtrip() {
        aux_test_short(0);
        aux_test_short(1);
        aux_test_short(SHORT_B1);
        aux_test_short(SHORT_B1 + 1);
        aux_test_short(SHORT_B2);
        aux_test_short(SHORT_B2 + 1);
        aux_test_short(SHORT_B3);
        aux_test_short(SHORT_B3 + 1);
        aux_test_short(SHORT_B4);
        aux_test_short(SHORT_B4 + 1);
        aux_test_short(VLE_U32_SHORT_VAL_MAX);

        for shift in 0..32 {
            aux_test_short(1u32 << shift);
            aux_test_short((1u32 << shift).wrapping_sub(1));
        }
    }

    #[test]
    fn test_short_encoding_convention() {
        let mut buf = [0u8; VLE_U32_SHORT_LEN_MAX];

        // 1-byte: values 0-251 stored directly
        let s = serialize_vint_u32_short(0, &mut buf);
        assert_eq!(s, &[0u8]);

        let s = serialize_vint_u32_short(1, &mut buf);
        assert_eq!(s, &[1u8]);

        let s = serialize_vint_u32_short(251, &mut buf);
        assert_eq!(s, &[251u8]);

        // 2-byte: tag 252
        let s = serialize_vint_u32_short(252, &mut buf);
        assert_eq!(s[0], 252);
        assert_eq!(s.len(), 2);

        // 3-byte: tag 253
        let s = serialize_vint_u32_short(SHORT_B2 + 1, &mut buf);
        assert_eq!(s[0], 253);
        assert_eq!(s.len(), 3);

        // 4-byte: tag 254
        let s = serialize_vint_u32_short(SHORT_B3 + 1, &mut buf);
        assert_eq!(s[0], 254);
        assert_eq!(s.len(), 4);

        // 5-byte: tag 255
        let s = serialize_vint_u32_short(SHORT_B4 + 1, &mut buf);
        assert_eq!(s[0], 255);
        assert_eq!(s.len(), 5);
    }

    #[test]
    fn test_short_len() {
        assert_eq!(vle_u32_short_len(0), 1);
        assert_eq!(vle_u32_short_len(251), 1);
        assert_eq!(vle_u32_short_len(252), 2);
        assert_eq!(vle_u32_short_len(SHORT_B2), 2);
        assert_eq!(vle_u32_short_len(SHORT_B2 + 1), 3);
        assert_eq!(vle_u32_short_len(SHORT_B3), 3);
        assert_eq!(vle_u32_short_len(SHORT_B3 + 1), 4);
        assert_eq!(vle_u32_short_len(SHORT_B4), 4);
        assert_eq!(vle_u32_short_len(SHORT_B4 + 1), 5);
        assert_eq!(vle_u32_short_len(VLE_U32_SHORT_VAL_MAX), 5);
    }

    #[test]
    #[should_panic(expected = "empty buffer")]
    fn test_short_decode_empty() {
        decode_vint_u32_short(&[]);
    }
}
