use std::io;
use std::io::{Read, Write};

use super::BinarySerializable;

/// Maximum encoded length of a VLE u64: 9 bytes.
///
/// The first 8 bytes each carry 7 data bits (56 total). The 9th byte carries
/// all 8 data bits (the decoder exits its loop at shift position 56 and uses
/// the full byte value), giving 56 + 8 = 64 bits total.
const VLE_LEN_MAX: usize = vle_len(u64::MAX);

const NEXT_BIT: u8 = 1<< 7;
const BYTE1_MASK: u64 = u64::MAX << 7;
const BYTE2_MASK: u64 = u64::MAX << (7 * 2);
const BYTE3_MASK: u64 = u64::MAX << (7 * 3);
const BYTE4_MASK: u64 = u64::MAX << (7 * 4);
const BYTE5_MASK: u64 = u64::MAX << (7 * 5);
const BYTE6_MASK: u64 = u64::MAX << (7 * 6);
const BYTE7_MASK: u64 = u64::MAX << (7 * 7);
const BYTE8_MASK: u64 = u64::MAX << (7 * 8);

/// Returns the number of bytes needed to encode `x` as a variable-length integer.
pub const fn vle_len(x: u64) -> usize {
    if (x & BYTE1_MASK) == 0 {
        1
    } else if (x & BYTE2_MASK) == 0 {
        2
    } else if (x & BYTE3_MASK) == 0 {
        3
    } else if (x & BYTE4_MASK) == 0 {
        4
    } else if (x & BYTE5_MASK) == 0 {
        5
    } else if (x & BYTE6_MASK) == 0 {
        6
    } else if (x & BYTE7_MASK) == 0 {
        7
    } else if (x & BYTE8_MASK) == 0 {
        8
    } else {
        9
    }
}

/// Wrapper over a `u128` that serializes as two VLE-encoded `u64`s (low, high).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VIntU128(pub u128);

impl BinarySerializable for VIntU128 {
    fn serialize<W: Write + ?Sized>(&self, writer: &mut W) -> io::Result<()> {
        VInt(self.0 as u64).serialize(writer)?;
        VInt((self.0 >> u64::BITS) as u64).serialize(writer)
    }

    fn deserialize<R: Read>(reader: &mut R) -> io::Result<Self> {
        let lo = VInt::deserialize(reader)?.0 as u128;
        let hi = VInt::deserialize(reader)?.0 as u128;
        Ok(VIntU128(lo | (hi << u64::BITS)))
    }
}

/// Wrapper over a `u64` that serializes as a variable-length integer.
///
/// Uses thubo's VLE convention: bit 7 (`0x80`) is set on continuation bytes,
/// clear on the final byte. The 9th byte (at maximum encoding length) uses
/// all 8 bits as data since the decoder knows it is the last byte.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VInt(pub u64);

/// Serializes a `u32` as a variable-length integer into a fixed buffer.
/// Returns the slice of `buf` containing the encoded bytes.
#[inline]
pub fn serialize_vint_u32(val: u32, buf: &mut [u8; 5]) -> &[u8] {
    let mut tmp = [0u8; 9];
    let len = VInt(val as u64).serialize_into(&mut tmp);
    buf[..len].copy_from_slice(&tmp[..len]);
    &buf[..len]
}

/// Reads a VLE `u32` from a buffer and advances past the consumed bytes.
///
/// # Panics
///
/// If the buffer does not start with a valid VLE payload.
pub fn read_u32_vint(data: &mut &[u8]) -> u32 {
    VInt::deserialize(data).expect("Corrupted data. Invalid VLE u32").0 as u32
}

/// Reads a VLE `u32` from a buffer without advancing.
/// Returns the decoded value and the number of bytes consumed.
pub fn read_u32_vint_no_advance(data: &[u8]) -> (u32, usize) {
    let vint = VInt::deserialize(&mut &data[..]).expect("Corrupted data. Invalid VLE u32");
    (vint.0 as u32, vle_len(vint.0))
}

/// Writes a `u32` as a VLE payload.
pub fn write_u32_vint<W: io::Write + ?Sized>(val: u32, writer: &mut W) -> io::Result<()> {
    VInt(val as u64).serialize(writer)
}

impl VInt {
    pub fn val(&self) -> u64 {
        self.0
    }

    pub fn deserialize_u64<R: Read>(reader: &mut R) -> io::Result<u64> {
        VInt::deserialize(reader).map(|vint| vint.0)
    }

    pub fn serialize_into_vec(&self, output: &mut Vec<u8>) {
        let mut buffer = [0u8; 9];
        let num_bytes = self.serialize_into(&mut buffer);
        output.extend_from_slice(&buffer[..num_bytes]);
    }

    pub fn serialize_into(&self, buffer: &mut [u8; 9]) -> usize {
        let x = self.0;

        buffer[0] = x as u8;
        if (x & BYTE1_MASK) == 0 { return 1; }
        buffer[0] |= NEXT_BIT;

        buffer[1] = (x >> 7) as u8;
        if (x & BYTE2_MASK) == 0 { return 2; }
        buffer[1] |= NEXT_BIT;

        buffer[2] = (x >> 14) as u8;
        if (x & BYTE3_MASK) == 0 { return 3; }
        buffer[2] |= NEXT_BIT;

        buffer[3] = (x >> 21) as u8;
        if (x & BYTE4_MASK) == 0 { return 4; }
        buffer[3] |= NEXT_BIT;

        buffer[4] = (x >> 28) as u8;
        if (x & BYTE5_MASK) == 0 { return 5; }
        buffer[4] |= NEXT_BIT;

        buffer[5] = (x >> 35) as u8;
        if (x & BYTE6_MASK) == 0 { return 6; }
        buffer[5] |= NEXT_BIT;

        buffer[6] = (x >> 42) as u8;
        if (x & BYTE7_MASK) == 0 { return 7; }
        buffer[6] |= NEXT_BIT;

        buffer[7] = (x >> 49) as u8;
        if (x & BYTE8_MASK) == 0 { return 8; }
        buffer[7] |= NEXT_BIT;

        buffer[8] = (x >> 56) as u8;
        9
    }
}

impl BinarySerializable for VInt {
    fn serialize<W: Write + ?Sized>(&self, writer: &mut W) -> io::Result<()> {
        let mut buffer = [0u8; 9];
        let num_bytes = self.serialize_into(&mut buffer);
        writer.write_all(&buffer[..num_bytes])
    }

    #[allow(clippy::unbuffered_bytes)]
    fn deserialize<R: Read>(reader: &mut R) -> io::Result<Self> {
        #[allow(clippy::unbuffered_bytes)]
        let mut bytes = reader.bytes();

        let mut b = match bytes.next() {
            Some(Ok(b)) => b,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Reached end of buffer while reading VInt",
                ));
            }
        };

        let mut v = 0u64;
        let mut i = 0usize;
        while (b & NEXT_BIT) != 0 && i != 7 * (VLE_LEN_MAX - 1) {
            v |= ((b & 0x7f) as u64) << i;
            b = match bytes.next() {
                Some(Ok(b)) => b,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Reached end of buffer while reading VInt",
                    ));
                }
            };
            i += 7;
        }
        v |= (b as u64) << i;
        Ok(VInt(v))
    }
}

#[cfg(test)]
mod tests {
    use super::{BinarySerializable, VInt, VIntU128, NEXT_BIT, VLE_LEN_MAX, serialize_vint_u32, vle_len};

    fn aux_test_vint(val: u64) {
        let mut v = [14u8; 9];
        let num_bytes = VInt(val).serialize_into(&mut v);
        for el in &v[num_bytes..] {
            assert_eq!(el, &14u8);
        }
        assert!(num_bytes > 0);
        assert_eq!(num_bytes, vle_len(val));
        if num_bytes < VLE_LEN_MAX {
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
        let mut buffer = [0u8; 9];
        let mut buffer2 = [0u8; 5];
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

    #[test]
    fn test_vle_encoding_convention() {
        let mut buf = [0u8; 9];

        let len = VInt(0).serialize_into(&mut buf);
        assert_eq!(&buf[..len], &[0x00]);

        let len = VInt(127).serialize_into(&mut buf);
        assert_eq!(&buf[..len], &[0x7f]);

        let len = VInt(128).serialize_into(&mut buf);
        assert_eq!(&buf[..len], &[NEXT_BIT, 0x01]);

        let len = VInt(16384).serialize_into(&mut buf);
        assert_eq!(&buf[..len], &[NEXT_BIT, NEXT_BIT, 0x01]);

        // 300 = 0b100101100 -> [0xAC, 0x02]
        let len = VInt(300).serialize_into(&mut buf);
        assert_eq!(&buf[..len], &[0xAC, 0x02]);

        // u64::MAX encodes as 9 bytes (all 0xFF)
        let len = VInt(u64::MAX).serialize_into(&mut buf);
        assert_eq!(len, 9);
        assert_eq!(&buf[..len], &[0xFF; 9]);

        // 2^63 encodes as 9 bytes: all NEXT_BIT
        let len = VInt(1u64 << 63).serialize_into(&mut buf);
        assert_eq!(len, 9);
        assert_eq!(&buf[..len], &[NEXT_BIT; 9]);
    }

    #[test]
    fn test_vle_len_max() {
        assert_eq!(VLE_LEN_MAX, 9);
        assert_eq!(vle_len(0), 1);
        assert_eq!(vle_len(127), 1);
        assert_eq!(vle_len(128), 2);
        assert_eq!(vle_len(u32::MAX as u64), 5);
        assert_eq!(vle_len(u64::MAX), 9);
        assert_eq!(vle_len(1u64 << 63), 9);
        assert_eq!(vle_len((1u64 << 56) - 1), 8);
        assert_eq!(vle_len(1u64 << 56), 9);
    }

    fn aux_test_vint_u128(val: u128) {
        let mut buf = Vec::new();
        VIntU128(val).serialize(&mut buf).unwrap();
        let decoded = VIntU128::deserialize(&mut &buf[..]).unwrap();
        assert_eq!(val, decoded.0);
    }

    #[test]
    fn test_vint_u128() {
        aux_test_vint_u128(0);
        aux_test_vint_u128(1);
        aux_test_vint_u128(u64::MAX as u128);
        aux_test_vint_u128(u64::MAX as u128 + 1);
        aux_test_vint_u128(u128::MAX);
        aux_test_vint_u128((1u128 << 64) - 1);
        aux_test_vint_u128(1u128 << 64);
        aux_test_vint_u128(1u128 << 127);
        aux_test_vint_u128(340_282_366_920_938_463_463_374_607_431_768_211_455); // u128::MAX
    }
}
