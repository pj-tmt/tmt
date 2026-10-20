//! Dispatch operation IDs: a digest of what the operation is about, shaped as a UUID.

/// The first 16 bytes of `digest` as an RFC 9562 custom (version 8) UUID. Callers hash their own
/// fixed domain and identities, so the same operation always gets the same ID.
pub fn uuid_v8(digest: &[u8]) -> String {
    let mut bytes: [u8; 16] = digest[..16]
        .try_into()
        .expect("a digest of at least 16 bytes");
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}
