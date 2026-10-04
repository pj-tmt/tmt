---
name: tmt-colab
description: Build, run and verify Colab (`tmt-colab` executable, `tmt colab ...`, the local page app, the browser client and the native packaging path). Load when changing extensions/tmt-colab or running its checks. Owner - the tmt-colab squad.
---

# Colab development

Behavior and wire shapes are owned by
[`extensions/tmt-colab/contracts/colab-v1.md`](../../../extensions/tmt-colab/contracts/colab-v1.md)
and [ARCHITECTURE.md](../../../ARCHITECTURE.md); this skill covers how to build, run,
package and verify. Shared gates are in [DEVELOPMENT.md](../../../DEVELOPMENT.md).
Colab and Remote share the state leaf: see [tmt-remote](../tmt-remote/SKILL.md).

Always use isolated data roots in tests; never point them at the real TMT directory.
Product limits are named in `src/limits.rs`, not in guides.

## References

- [references/development.md](references/development.md): focused test commands per
  area, page source CLI, app and browser-client builds, packaging and archive
  verification.
- [references/architecture-state.md](references/architecture-state.md): module layout,
  gotchas, persistence, owner transitions and admission.
- [references/architecture-sync-decoder.md](references/architecture-sync-decoder.md):
  stream sync, isolated decoder, page source and export, browser containment.
- [references/acceptance.md](references/acceptance.md): the Ask agent real-binary
  acceptance suite and its harness.
- [references/ask-agent.md](references/ask-agent.md): the browser-direct Ask agent
  modules, invariants and tests.
- [references/discussion.md](references/discussion.md): typed threads/comments,
  cosmetic quote anchors, parent actions and verification.

- [references/page-chrome.md](references/page-chrome.md): frameless page viewport,
  single-row chrome, persistent drawers and responsive layout verification.
