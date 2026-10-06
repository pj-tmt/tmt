//! Pure private arithmetic and one-stream gzip admission controls.
use super::*;
use flate2::{Compression, write::GzEncoder};
use std::io::Write;

#[test]
fn checked_total_accepts_empty_and_exact_usize_max_and_rejects_overflow() {
    assert_eq!(checked_bytes([].into_iter()).unwrap(), 0);
    assert_eq!(
        checked_bytes([usize::MAX - 1, 1].into_iter()).unwrap(),
        usize::MAX
    );
    let error = checked_bytes([usize::MAX, 1].into_iter()).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<OwnerFault>(),
        Some(OwnerFault::Capacity)
    ));
}
fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(6));
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}
#[test]
fn gzip_exact_budget_and_one_byte_over_use_one_stream() {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let noise = (0..5_000_001)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect::<Vec<_>>();
    // Establish the exact threshold with complete compressed output, independently of the counting sink.
    let mut low = 4_990_000usize;
    let mut high = 5_000_000usize;
    while low < high {
        let mid = (low + high) / 2;
        if gzip(&noise[..mid]).len() < 5_000_000 {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    let exact = &noise[..low];
    assert_eq!(gzip(exact).len(), 5_000_000);
    let over = &noise[..low + 1];
    assert_eq!(gzip(over).len(), 5_000_001);
    assert_eq!(gzip_over_budget([exact].into_iter()).unwrap(), None);
    let split = low / 2;
    assert_eq!(
        gzip_over_budget([&exact[..split], &exact[split..]].into_iter()).unwrap(),
        None
    );
    assert_eq!(
        gzip_over_budget([&over[..split], &over[split..]].into_iter()).unwrap(),
        Some(5_000_001)
    );
}
