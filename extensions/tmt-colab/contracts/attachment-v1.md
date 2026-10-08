# Attachment descriptors and manifests

This is the bounded Colab byte grammar introduced by #1853's first slice.
The model/client codecs verify syntax and exact asset cryptography; callers still
own authenticated reference, current membership, historical-key and publication
admission. The [Storage proposal](storage-v1-proposal.md) owns the remaining
content-access/channel implementation. This grammar does not activate Remote,
upload UI, snapshot creation or a new filesystem command.

## Descriptor

A descriptor is strict UTF-8 JSON, at most 2,048 encoded bytes. Missing, duplicate
or unknown fields are rejected. Canonical serialization uses compact JSON with
this field order, Unicode encoded as UTF-8 and ordinary JSON string escaping:

`version,attachmentId,space,page,epoch,namespace,objectId,authorDevice,membershipRevision,source,envelopeHash,signature,payloadSha256,payloadBytes,plaintextBytes,filename,mediaType`.

- `version` is integer 1. IDs, space, positive decimal epochs/revisions and
  namespaces use existing Colab validators. Object IDs and both hashes are
  lowercase 64-character hex. `signature` is canonical base64url of 64 bytes.
- `source` retains original provenance, with exactly `{kind:"document",sourceDigest}`
  (lowercase SHA-256 of original source bytes, namespace `content`) or
  `{kind:"message",writerId,messageId,messageRevision}` (canonical UUIDs and
  positive decimal revision, namespace `own`). Final references authenticate the
  descriptor hash; a containing envelope is not hashed into its own descriptor.
- `envelopeHash` is the existing framed Colab envelope hash. `payloadSha256`
  hashes the exact serialized encrypted envelope bytes, including JSON syntax.
  These are distinct commitments. `payloadBytes` is positive decimal text at
  most 12 MiB; `plaintextBytes` is nonnegative decimal text at most 8 MiB.
- `filename` is nonempty UTF-8, at most 255 bytes, without control characters.
  `mediaType` is at most 128 bytes: two nonempty lowercase ASCII tokens separated
  by `/`; token characters are letters, digits and `!#$&^_.+-`. These are inert
  display labels, not MIME handlers, paths or permissions.

The canonical descriptor commitment is SHA-256 of existing
u32-length framing with domain `tmt-colab-attachment-v1`, `"1"`, then the fields
above after `version`, in order. All fields are UTF-8 text except `source`, which
is itself length-framed: `"document",sourceDigest` or
`"message",writerId,messageId,messageRevision`. The descriptor carries the
existing asset signature; it introduces no signer or authority root.

Asset opening checks the exact caller-admitted asset context, object ID, framed
hash/signature, raw serialized digest/length, existing AES-GCM/Ed25519 verification
and resulting plaintext length. Its header remains `kind:asset`, zero sequence
and zero previous hash. The caller must supply an independently admitted creator
key, eligible epoch secret and authenticated descriptor/reference; knowing an
object ID or possessing a server epoch secret establishes none of those.

## Containing projections and manifest

Document `meta.attachments` is an optional ordered list of at most 128 descriptors.
Chat and annotation comment records share optional `attachments`, at most 16;
each descriptor must match the record's space/page. Attachment IDs cannot repeat
within a list. An undeleted comment may have empty text when attachments exist;
a deleted comment has empty text and no attachments. Absent fields retain the
existing no-attachment grammar. Source edits, checkpoints and epoch baselines
preserve admitted document descriptors without embedding any binary bodies.

A strict snapshot manifest is
`{version,space,page,snapshotId,authorDevice,membershipRevision,sourceDigest,attachments}`,
with integer version 1, existing canonical scope/IDs/revision, lowercase source
SHA-256 and at most 128 unique descriptors of the exact same space/page. Its JSON
ceiling is `128 * 2048 + 1024` bytes. It retains original source bytes/digest and
original descriptor provenance; no latest-content substitution is allowed.

The manifest commitment is SHA-256 of u32-length framing of
`"tmt-colab-attachment-manifest-v1","1",space,page,snapshotId,authorDevice,membershipRevision,sourceDigest,list`.
`list` is a four-byte big-endian count followed by length-framed canonical
**descriptor inputs**, in their exact manifest order. The subsequent content
admission slice binds this hash to the authenticated snapshot record. Parsing a
manifest alone never creates a snapshot or permits a historical read.

`vectors/attachment-reference.py` independently frames, encrypts and signs public
fixtures, freezing `attachment-v1.json`. Native model/decoder and browser
client/Worker tests consume that one corpus; the client differential gate also
executes its codec/crypto cases in Chromium, Firefox and WebKit. The corpus does
not stand in for the later live callback, committed-publication or reader-policy
controls.
