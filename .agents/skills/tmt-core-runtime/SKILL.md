---
name: tmt-core-runtime
description: Per-module architecture knowledge for the core Rust runtime - identity and bindings, sessions and provider hooks, SQLite storage and requests, hosts and drivers, provider channels, extension API/hooks, managed skills and native installation. Load when changing rust/crates/tmt-core, tmt-adapters or tmt-cli behavior in those areas. Owner - the tmt-core squad.
---

# Core runtime

[ARCHITECTURE.md](../../../ARCHITECTURE.md) holds the cross-cutting map (layers,
dependency direction, owner per concern). This skill holds the module-level rules a
change in that code must preserve. Public shapes and limits live in
[`contracts/`](../../../contracts/); verification gates are in
[DEVELOPMENT.md](../../../DEVELOPMENT.md) and [tmt-dev](../tmt-dev/SKILL.md). When a rule
here and the code disagree, fix the doc in the same PR.

## References

Load the file for the area you change.

- [references/identity-bindings.md](references/identity-bindings.md): names, binding
  evidence and process incarnation, session/driver state, activity and consumption,
  provider hooks, setup and uninstall, foreground launch.
- [references/requests-storage.md](references/requests-storage.md): SQLite adapter and
  migration rules (including the change-cursor trigger rule), the request service, reply
  notices and configuration.
- [references/hosts-drivers.md](references/hosts-drivers.md): process and tmux effects, the
  host port, external host drivers, agent drivers, provider channels, driver protocol.
- [references/extension-surface.md](references/extension-surface.md): command dispatch and
  help, external commands, local API, MCP and extension hooks.
- [references/install-architecture.md](references/install-architecture.md): managed skills,
  native installation, companions, `tmt extension` and CLI self-upgrade.
  Pipeline procedures are in [tmt-release](../tmt-release/SKILL.md).

Record a new module rule only when it is a maintained invariant a reader cannot learn
from the code or its tests; keep history, issue numbers and measurements in the PR.
