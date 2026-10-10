# Digest extension contract v1

Digest owns individual member settings. It uses public Core commands for identity
and configuration-path discovery; it never reads Core storage or configuration files.

## Settings

`tmt digest <member> <duration>|auto|off|default` resolves a saved identity name or
exact UUID. Values are positional. Digest imposes no squad or lead restriction.
`default` removes the override and its current attribution; the member then inherits
the global default. Explicit `auto` remains an override even when the global default
is a duration. `off` disables the member's configured digest mode.

The file is `digest.toml` beside the global configuration file reported by public
`tmt config show --json` (normally `~/.config/tmt/digest.toml`). Missing files inherit
the shipped defaults. Global `default` and `flushCount` are TOML-only, with no CLI flags:

```toml
default = "auto"
flushCount = 10

[members.10000000-0000-4000-8000-000000000001]
mode = "20s"
setByIdentityId = "20000000-0000-4000-8000-000000000001"
setAtMs = 1791629000000
```

`default` and member `mode` accept `auto`, `off` or a positive duration. Durations
use ms/s/m/h/d, or bare seconds; decimals have at most nine fractional digits and
must resolve exactly to at least one
millisecond and at most 9,007,199,254,740,991 milliseconds. `flushCount` is a positive
integer (zero is invalid), defaults to 10, and applies only to interval mode.
Member keys and setter IDs are canonical identity UUIDs.

`setByIdentityId` is absent when Core cannot identify the setter. `setAtMs` is the
current override write timestamp, a nonnegative JS-safe integer; it is neither
delivery eligibility nor telemetry freshness. Inherited settings have no member
attribution and never borrow a removed setter or invent global attribution.

Writes validate the whole existing document, preserve comments and unknown fields,
serialize extension writers, and atomically replace the file with private permissions.
Invalid input, invalid files and failed Core discovery leave settings unchanged.

The extension remains unpublished. Member writes save configuration; the
[minute tick](digest-tick-v1.md) applies it and offers Core delivery opportunities.
No skill or managed installation is shipped by this settings surface.
