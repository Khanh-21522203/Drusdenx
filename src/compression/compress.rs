use crate::compression::delta::DeltaEncoder;
use crate::compression::vbyte::VByteEncoder;
use crate::core::error::{Error, ErrorKind, Result};
use serde::{Serialize, Deserialize};

/// Compressed block storage for general purpose data
#[derive(Serialize, Deserialize)]
pub struct CompressedBlock {
    pub data: Vec<u8>,
    pub original_size: usize,
    pub compression: CompressionType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CompressionType {
    None,
    LZ4,      // Fast compression (~500 MB/s), ratio 2-3x
    Zstd,     // Better ratio (3-5x), slower (~200 MB/s)
    Snappy,   // Balanced (2-3x ratio, ~300 MB/s)
}

impl CompressedBlock {
    /// Compress raw byte data (for text, binary)
    pub fn compress(data: &[u8], compression: CompressionType) -> Result<Self> {
        let compressed = match compression {
            CompressionType::None => data.to_vec(),

            CompressionType::LZ4 => {
                lz4::block::compress(data, None, false)?
            }

            CompressionType::Zstd => {
                zstd::encode_all(data, 3)?  // Level 3 is balanced
            }

            CompressionType::Snappy => {
                use snap::raw::Encoder;
                let mut encoder = Encoder::new();
                encoder.compress_vec(data)
                    .map_err(|e| Error::new(ErrorKind::Io, e.to_string()))?
            }
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

            CompressionType::Zstd => {
                zstd::decode_all(&self.data[..])
                    .map_err(|e| Error::new(ErrorKind::Io, e.to_string()))
            }

            CompressionType::Snappy => {
                use snap::raw::Decoder;
                let mut decoder = Decoder::new();
                decoder.decompress_vec(&self.data)
                    .map_err(|e| Error::new(ErrorKind::Io, e.to_string()))
            }
        }
    }

    /// Choose compression based on use case
    pub fn compress_auto(data: &[u8], priority: CompressionPriority) -> Result<Self> {
        let compression = match priority {
            CompressionPriority::Speed => CompressionType::LZ4,      // Fastest
            CompressionPriority::Ratio => CompressionType::Zstd,     // Best compression
            CompressionPriority::Balanced => CompressionType::Snappy, // Middle ground
        };
        Self::compress(data, compression)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum CompressionPriority {
    Speed,     // Use LZ4 - indexing, hot data
    Ratio,     // Use Zstd - cold data, archival
    Balanced,  // Use Snappy - general purpose
}

pub struct EncodedIntegerBlock {
    pub data: Vec<u8>,
    pub original_count: usize,  // Number of integers
    pub encoding: IntegerEncodingType,
}

#[derive(Debug, Clone, Copy)]
pub enum IntegerEncodingType {
    None,      // Raw u32 array (4 bytes each)
    Delta,     // Delta encoding - best for SORTED integers (doc IDs)
    VByte,     // Variable byte - best for SMALL integers (positions, term freq)
    // Note: For general compression, apply LZ4/Zstd AFTER encoding
}

impl EncodedIntegerBlock {
    /// Encode array of u32 integers using specialized integer encoding
    pub fn encode(nums: &[u32], encoding: IntegerEncodingType) -> Result<Self> {
        let encoded = match encoding {
            IntegerEncodingType::None => {
                // Raw format: 4 bytes per integer
                let mut bytes = Vec::with_capacity(nums.len() * 4);
                for &num in nums {
                    bytes.extend_from_slice(&num.to_le_bytes());
                }
                bytes
            }
            IntegerEncodingType::Delta => {
                // Delta encoding: stores differences
                DeltaEncoder::encode_u32_list(nums)?
            }
            IntegerEncodingType::VByte => {
                // Variable byte: 1-5 bytes per integer
                VByteEncoder::encode_u32_list(nums)?
            }
        };

        Ok(EncodedIntegerBlock {
            data: encoded,
            original_count: nums.len(),
            encoding,
        })
    }

    pub fn decode(&self) -> Result<Vec<u32>> {
        match self.encoding {
            IntegerEncodingType::None => {
                Ok(self.data.chunks_exact(4)
                    .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect())
            }
            IntegerEncodingType::Delta => DeltaEncoder::decode_u32_list(&self.data),
            IntegerEncodingType::VByte => VByteEncoder::decode_u32_list(&self.data),
        }
    }

    /// Optional: Apply general compression AFTER integer encoding
    /// This gives best results: Delta/VByte first, then LZ4
    pub fn compress_with_lz4(&self) -> Result<CompressedBlock> {
        CompressedBlock::compress(&self.data, CompressionType::LZ4)
    }
}