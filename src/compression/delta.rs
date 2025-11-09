use crate::compression::vbyte::VByteEncoder;
use crate::core::error::Result;

/// Delta encoding for sorted integers (best for doc IDs)
pub struct DeltaEncoder;

impl DeltaEncoder {
    /// Encode u32 array with delta encoding
    /// Best for sorted integers like doc IDs
    pub fn encode_u32_list(nums: &[u32]) -> Result<Vec<u8>> {
        if nums.is_empty() {
            return Ok(Vec::new());
        }

        let mut output = Vec::new();
        // Store first value as-is
        output.extend_from_slice(&nums[0].to_le_bytes());

        // Store deltas using VByte encoding
        for i in 1..nums.len() {
            let delta = nums[i].wrapping_sub(nums[i - 1]);
            VByteEncoder::encode_u32(&mut output, delta)?;
        }

        Ok(output)
    }

    /// Decode to u32 array
    pub fn decode_u32_list(data: &[u8]) -> Result<Vec<u32>> {
        if data.len() < 4 {
            return Ok(Vec::new());
        }

        // Read first value
        let first = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let mut nums = vec![first];
        let mut pos = 4;
        let mut prev = first;

        // Decode deltas
        while pos < data.len() {
            let (delta, consumed) = VByteEncoder::decode_u32(&data[pos..])?;
            let val = prev.wrapping_add(delta);
            nums.push(val);
            prev = val;
            pos += consumed;
        }

        Ok(nums)
    }
}