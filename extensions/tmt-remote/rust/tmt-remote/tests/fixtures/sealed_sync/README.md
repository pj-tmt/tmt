# Sealed-object vectors

Remote-authored synthetic data, not Colab declarations or provider evidence.
`v1.json` freezes decoded-byte sizes, original-derived IDs and canonical manifest
bytes for both declared chunk profiles. Payload byte i is i modulo 251. Expected
SHA-256 values and JSON were produced independently with Python hashlib and
json.dumps(sort_keys=True, separators=(",", ":")), not either implementation.
Rust and SDK read this same corpus. No credential, encryption key or real project
is represented. Empty objects publish a zero-chunk manifest; nonempty objects have
full nonfinal chunks and one bounded final chunk.

`codec-v1.json` freezes a synthetic extension declaration, provider fields bytes and
positive/negative restricted Value cases. Field names intentionally differ from
Colab's to detect hard-coded mappings. Document/declaration hashes use Python hashlib.
Safe integers, canonical padded base64 and exact microsecond UTC timestamps are
covered; extra precision and unsupported nested/provider values are refused.

The object manifest vectors are logical object data, not an alternate wire codec.
An adapter adds declared admission metadata and encodes those fields through the
restricted REST codec; no extra envelope or production manifest producer ships here.
