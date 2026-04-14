/// Test actual header size
use Drusdenx::storage::segment::SegmentHeader;

fn main() {
    let header = SegmentHeader::new(1);
    let serialized = bincode::serialize(&header).unwrap();
    
    println!("SegmentHeader::SIZE = {}", SegmentHeader::SIZE);
    println!("Actual serialized size = {}", serialized.len());
    println!("Match: {}", SegmentHeader::SIZE == serialized.len());
}
