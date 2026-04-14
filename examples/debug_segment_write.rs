/// Debug segment write/read
use Drusdenx::core::types::{Document, DocId, FieldValue};
use Drusdenx::storage::layout::StorageLayout;
use Drusdenx::storage::segment::{SegmentId, SegmentHeader};
use Drusdenx::storage::segment_writer::SegmentWriter;
use Drusdenx::storage::segment_reader::SegmentReader;
use Drusdenx::memory::buffer_pool::BufferPool;
use std::path::PathBuf;
use std::sync::Arc;
use std::fs;
use std::io::{Read, Seek, SeekFrom};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let storage_path = PathBuf::from("/tmp/test_seg");
    let storage = StorageLayout::new(storage_path.clone())?;
    
    let segment_id = SegmentId::new();
    println!("=== Writing Segment ===");
    println!("Segment ID: {:?}", segment_id);
    
    let buffer_pool = Arc::new(BufferPool::new(1024 * 1024));
    let mut writer = SegmentWriter::new(&storage, segment_id, buffer_pool)?;
    
    // Add a document
    let mut doc = Document::new(DocId(1));
    doc.fields.insert("content".to_string(), FieldValue::Text("rust programming".to_string()));
    
    println!("Writing document...");
    writer.write_document(&doc)?;
    
    println!("Finishing segment...");
    let segment = writer.finish(&storage)?;
    println!("Segment written: doc_count={}", segment.doc_count);
    
    // Check file size
    let seg_path = storage.segment_path(&segment_id);
    let metadata = fs::metadata(&seg_path)?;
    println!("\nFile size: {} bytes", metadata.len());
    
    // Hex dump first 100 bytes
    let mut file = fs::File::open(&seg_path)?;
    let mut buffer = vec![0u8; 100.min(metadata.len() as usize)];
    file.read_exact(&mut buffer)?;
    
    println!("\nFirst {} bytes (hex):", buffer.len());
    for (i, byte) in buffer.iter().enumerate() {
        if i % 16 == 0 {
            print!("\n{:04x}: ", i);
        }
        print!("{:02x} ", byte);
    }
    println!("\n");
    
    // Try to read it back
    println!("=== Reading Segment ===");
    file.seek(SeekFrom::Start(0))?;
    
    let header: SegmentHeader = bincode::deserialize_from(&mut file)?;
    let pos_after_header = file.seek(SeekFrom::Current(0))?;
    println!("Header: version={}, doc_count={}, checksum={}", 
        header.version, header.doc_count, header.checksum);
    println!("File position after header: {}", pos_after_header);
    
    // Read first length
    let mut len_buf = [0u8; 4];
    file.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf);
    println!("First document length: {} bytes", len);
    
    if len > 0 && len < 10000 {
        println!("✅ Document length looks valid!");
    } else {
        println!("❌ Document length looks wrong!");
    }
    
    Ok(())
}
