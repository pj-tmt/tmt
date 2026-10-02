# Archived #830 hostile decoder evidence

Archive: `~/dev/tmux-team-colab-archive/830-colab-local-2026-10-01.tar.gz`.
Archive SHA-256: `feb5d1e3d3f35f7630ffb81b39d4558ecd98664c0cba3943f08e79de2a062d60`.

The six dumps are unchanged archive bytes: case 26 timed out; cases 60, 106,
147, 157 and 192 panicked with yrs 0.28.0 and Rust 1.95 in #830.
`generator.rs` is the exact original evidence generator, not production code.
`valid.bin` is `js-input.json` update 0; it uses the old `annotations` root,
so it is a CRDT generator seed rather than an admitted Colab document.

The integration test reproduces seed `0x830c01ab` and the first 261 cases,
checks all six dumps byte for byte, and runs each case in a fresh child with
a one-second deadline and a 45-second suite budget, twice. On Linux, case 26
may instead exit unsuccessfully with an allocation-failure or panic diagnostic
under the enforced address-space limit; it must never succeed. Other platforms
retain the deadline requirement. Both outcomes must permit public runner reuse.
Production uses separate pinned budgets; the archive proves process containment, not a sandbox
or macOS memory containment. Valid content/own controls are separate tests.

| File           | SHA-256                                                            |
| -------------- | ------------------------------------------------------------------ |
| `106.bin`      | `c6557e71b6da6e6a0491f73491421d9f3dc6e48bedd333d4b004b5e482d02d32` |
| `147.bin`      | `38894190d6ece562b631ebf1eb57259d471ccb71405df257f3e449fa1aa6e1fb` |
| `157.bin`      | `3d9596ca1bc9561b76f9331a0fd24f9506094ef2a1343a66ea9548e4ef45f860` |
| `192.bin`      | `72e788684460294ac59d9cea6bbbd6654d9fc3d8e914954ed499d8a9c967f343` |
| `26.bin`       | `249541196bc1a9bd584f05b1c0dc8255c7b77b0b2f6b489abaf9b102f4187a4b` |
| `60.bin`       | `12d0ff2099a8e1dc696575c1d991dd9ce97fd48424cc4351288b7d26d55e8bb1` |
| `generator.rs` | `aef871130e700527e2d73e60a176b28d71cbf07a21b51625acc071b1a0370e5a` |
| `valid.bin`    | `9da57e25232d9d2e6342eacacd937ecc588a4e9e6d3634546dd3b1fc5315b92b` |
