# Sealed-object vectors

Remote-authored synthetic data, not Colab declarations or provider evidence.
`v1.json` freezes decoded-byte sizes, original-derived IDs and canonical manifest
bytes for both declared chunk profiles. Payload byte i is i modulo 251. Expected
SHA-256 values and JSON were produced independently with Python hashlib and
json.dumps(sort_keys=True, separators=(",", ":")), not either implementation.
Rust and SDK read this same corpus. No credential, encryption key or real project
is represented. Empty objects publish a zero-chunk manifest; nonempty objects have
full nonfinal chunks and one bounded final chunk.
