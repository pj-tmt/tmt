Remote-authored synthetic public configuration vector; no Colab or provider provenance.
The checksum was independently computed with Python hashlib over sorted compact JSON
with the checksum field omitted. This is corruption detection, never an access grant.

A JavaScript receiver reproduces the checksum by removing checksum, recursively sorting
object keys (including publicWebConfig), JSON.stringify without whitespace, then hashing
TextEncoder UTF-8 bytes with SHA-256 and encoding lowercase hex. All schema strings are
printable ASCII, so JSON escaping and UTF-8 agree with Rust; never hash pretty-printed JSON.
The complete descriptor uses the same canonical bytes and unpadded base64url encoding.
