# Colab model vectors

The [colab-v1 contract](../colab-v1.md) owns byte definitions and full L1 gates.
All seeds and secrets here are public test fixtures, never runtime inputs.

`ed25519-829.jsonl` retains all 148 cases from #829's
`evidence/ed25519-differential-vectors.json`; each line is an unchanged case value,
including nine accepted positives and mixed-order controls.
`encoding-829.jsonl` retains the ten signed `cases` and five pairing `macCases`
from #829's `evidence/encoding-vectors.json`. Their historical header/cut/chain
schemas do not cover colab-v1 namespace changes and must not be used as current
wire admission. Rust verifies their exact signatures/MACs and primitive framing.
Source evidence is read-only under `spikes/829-colab-crypto/` in the #829 worktree;
[the accepted report](https://github.com/wkh237/tmt/issues/829#issuecomment-5933403999)
records provenance and bounded conclusions.

`model-reference.py` independently implements RFC5869 and LP framing with Python
stdlib; Python cryptography supplies AES-GCM and Ed25519. It imports no Rust or
browser implementation. `model-v1.json` freezes the new namespace header and cut,
sign-in proof/possession, management digest, ciphertext and envelope hash. Python
is needed only for deliberate regeneration/checking; ordinary Rust tests need no
Python dependency. A different Python/OpenSSL crypto implementation provides
independent known answers, not proof of universal curve/renderer behavior.
Complete schemas, HPKE/link vectors, browser interoperability and the three-engine
missing-engine failure gate remain later L1 work.

`keys.jsonl` retains #829 device/owner/link/RFC9180 recipient public keys and
[RFC8032 test 1](https://www.rfc-editor.org/rfc/rfc8032#section-7.1) /
[RFC7748 section 6.1](https://www.rfc-editor.org/rfc/rfc7748#section-6.1) key KATs.
