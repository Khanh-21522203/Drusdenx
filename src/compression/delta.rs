use roaring::RoaringBitmap;
use crate::core::error::{Error, ErrorKind, Result};

/// Compressed block storage
pub struct CompressedBlock {
    pub data: Vec<u8>,
    pub original_size: usize,
    pub compression: CompressionType,
}

#[derive(Debug, Clone, Copy)]
pub enum CompressionType {
    None,
    LZ4,
}

#[derive(Debug, Clone, Copy)]
pub enum IntegerCompressionType {
    None,
    LZ4,
    Delta,
    VByte,
}

impl CompressedBlock {
    /// Compress raw byte data (for general purpose: text, binary)
    pub fn compress(data: &[u8], compression: CompressionType) -> Result<Self> {
        let compressed = match compression {
            CompressionType::None => data.to_vec(),
            CompressionType::LZ4 => lz4::block::compress(data, None, false)?,
        };

        Ok(CompressedBlock {
            data: compressed,
            original_size: data.len(),
            compression,
        })
    }

    pub fn decompress(&self) -> Result<Vec<u8>> {
        match self.compression {
            CompressionType::None => Ok(self.data.clone()),
            CompressionType::LZ4 => {
                lz4::block::decompress(&self.data, Some(self.original_size as i32))
                    .map_err(|e| Error::new(ErrorKind::Io, e.to_string()))
            }
        }
    }
}

/// Compressed integer block (for posting lists, doc IDs, positions)
pub struct CompressedIntegerBlock {
    pub data: Vec<u8>,
    pub original_count: usize,  // Number of integers, not bytes
    pub compression: IntegerCompressionType,
}

impl CompressedIntegerBlock {
    /// Compress array of u32 integers (for posting lists, doc IDs, etc.)
    pub fn compress(nums: &[u32], compression: IntegerCompressionType) -> Result<Self> {
        let compressed = match compression {
            IntegerCompressionType::None => {
                let mut bytes = Vec::with_capacity(nums.len() * 4);
                for &num in nums {
                    bytes.extend_from_slice(&num.to_le_bytes());
                }
                bytes
            }
            IntegerCompressionType::LZ4 => {
                let mut bytes = Vec::with_capacity(nums.len() * 4);
                for &num in nums {
                    bytes.extend_from_slice(&num.to_le_bytes());
                }
                lz4::block::compress(&bytes, None, false)?
            }
            IntegerCompressionType::Delta => DeltaEncoder::encode_u32_list(nums)?,
            IntegerCompressionType::VByte => VByteEncoder::encode_u32_list(nums)?,
        };

        Ok(CompressedIntegerBlock {
            data: compressed,
            original_count: nums.len(),
            compression,
        })
    }

    pub fn decompress(&self) -> Result<Vec<u32>> {
        match self.compression {
            IntegerCompressionType::None => {
                Ok(self.data.chunks_exact(4)
                    .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect())
            }
            IntegerCompressionType::LZ4 => {
                let bytes = lz4::block::decompress(&self.data, Some(self.original_count as i32 * 4))
                    .map_err(|e| Error::new(ErrorKind::Io, e.to_string()))?;
                Ok(bytes.chunks_exact(4)
                    .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect())
            }
            IntegerCompressionType::Delta => DeltaEncoder::decode_u32_list(&self.data),
            IntegerCompressionType::VByte => VByteEncoder::decode_u32_list(&self.data),
        }
    }
}

/// Delta encoding for sorted integers
pub struct DeltaEncoder;

impl DeltaEncoder {
    /// Encode bytes by treating as u32 array (legacy for compatibility)
    pub fn encode(data: &[u8]) -> Result<Vec<u8>> {
        let nums: Vec<u32> = data.chunks_exact(4)
            .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect();
        Self::encode_u32_list(&nums)
    }

    /// Encode u32 array with delta encoding
    pub fn encode_u32_list(nums: &[u32]) -> Result<Vec<u8>> {
        if nums.is_empty() {
            return Ok(Vec::new());
        }

        let mut output = Vec::new();
        output.extend_from_slice(&nums[0].to_le_bytes());

        for i in 1..nums.len() {
            let delta = nums[i].wrapping_sub(nums[i - 1]);
            VByteEncoder::encode_u32(&mut output, delta)?;
        }

        Ok(output)
    }

    /// Decode to bytes (legacy for compatibility)
    pub fn decode(data: &[u8]) -> Result<Vec<u8>> {
        let nums = Self::decode_u32_list(data)?;
        let mut output = Vec::with_capacity(nums.len() * 4);
        for num in nums {
            output.extend_from_slice(&num.to_le_bytes());
        }
        Ok(output)
    }

    /// Decode to u32 array
    pub fn decode_u32_list(data: &[u8]) -> Result<Vec<u32>> {
        if data.len() < 4 {
            return Ok(Vec::new());
        }

        let first = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let mut nums = vec![first];
        let mut pos = 4;
        let mut prev = first;

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

/// Variable byte encoding for integers
pub struct VByteEncoder;

impl VByteEncoder {
    /// Encode single u32 value
    pub fn encode_u32(output: &mut Vec<u8>, mut value: u32) -> Result<()> {
        while value >= 128 {
            output.push((value & 127) as u8 | 128);
            value >>= 7;
        }
        output.push(value as u8);
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

    /// Decode single u32 value
    pub fn decode_u32(input: &[u8]) -> Result<(u32, usize)> {
        let mut value = 0u32;
        let mut shift = 0;
        let mut consumed = 0;

        for byte in input {
            consumed += 1;
            value |= ((byte & 127) as u32) << shift;

            if byte & 128 == 0 {
                return Ok((value, consumed));
            }

            shift += 7;
            if shift > 28 {
                return Err(Error::new(ErrorKind::Parse, "VByte overflow".parse().unwrap()));
            }
        }

        Err(Error::new(ErrorKind::Parse, "Incomplete VByte".parse().unwrap()))
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

/// Roaring bitmap wrapper
pub struct CompressedBitmap(RoaringBitmap);

impl CompressedBitmap {
    pub fn new() -> Self {
        CompressedBitmap(RoaringBitmap::new())
    }

    pub fn insert(&mut self, value: u32) {
        self.0.insert(value);
    }

    pub fn intersect(&self, other: &CompressedBitmap) -> CompressedBitmap {
        CompressedBitmap(&self.0 & &other.0)
    }
}