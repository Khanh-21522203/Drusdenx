/// Test header sizes with different doc counts
use Drusdenx::storage::segment::SegmentHeader;

fn main() {
    let h0 = SegmentHeader::new(0);
    let h1 = SegmentHeader::new(1);
    let h1000 = SegmentHeader::new(1000);
    
    let s0 = bincode::serialize(&h0).unwrap();
    let s1 = bincode::serialize(&h1).unwrap();
    let s1000 = bincode::serialize(&h1000).unwrap();
    
    println!("Header with doc_count=0:    {} bytes", s0.len());
    println!("Header with doc_count=1:    {} bytes", s1.len());
    println!("Header with doc_count=1000: {} bytes", s1000.len());
    println!("\nAll same size? {}", s0.len() == s1.len() && s1.len() == s1000.len());
}
