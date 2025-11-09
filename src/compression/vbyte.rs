use crate::core::error::{Error, ErrorKind, Result};

/// Variable byte encoding for integers (best for small integers)
pub struct VByteEncoder;

impl VByteEncoder {
    /// Encode single u32 value
    /// Values < 128 use 1 byte, < 16384 use 2 bytes, etc.
    pub fn encode_u32(output: &mut Vec<u8>, mut value: u32) -> Result<()> {
        while value >= 128 {
            output.push((value & 127) as u8 | 128);  // Set continuation bit
            value >>= 7;
        }
        output.push(value as u8);  // Last byte without continuation bit
        Ok(())
    }

    /// Encode array of u32 values
    pub fn encode_u32_list(nums: &[u32]) -> Result<Vec<u8>> {
        let mut output = Vec::new();
        for &num in nums {
            Self::encode_u32(&mut output, num)?;
        }
        Ok(output)
    }

    /// Decode single u32 value, returns (value, bytes_consumed)
    pub fn decode_u32(input: &[u8]) -> Result<(u32, usize)> {
        let mut value = 0u32;
        let mut shift = 0;
        let mut consumed = 0;

        for &byte in input {
            consumed += 1;
            value |= ((byte & 127) as u32) << shift;

            if byte & 128 == 0 {  // No continuation bit
                return Ok((value, consumed));
            }

            shift += 7;
            if shift > 28 {  // Max 5 bytes for u32
                return Err(Error::new(ErrorKind::Parse, "VByte overflow".to_string()));
            }
        }

        Err(Error::new(ErrorKind::Parse, "Incomplete VByte".to_string()))
    }

    /// Decode array of u32 values
    pub fn decode_u32_list(data: &[u8]) -> Result<Vec<u32>> {
        let mut nums = Vec::new();
        let mut pos = 0;

        while pos < data.len() {
            let (value, consumed) = Self::decode_u32(&data[pos..])?;
            nums.push(value);
            pos += consumed;
        }

        Ok(nums)
    }
}