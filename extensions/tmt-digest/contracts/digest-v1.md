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

The extension remains unpublished. This command saves configuration; delivery ticks
are implemented separately before activation. No skill or managed installation is
shipped by this settings surface.

## Status document

This section defines the proposed `tmt digest status --json` reader contract for
phase 2; the help-only/settings surface above does not yet produce it. The frozen
[status vectors](vectors/status-v1.json) let consumers build against the contract
before the status writer ships. Ops and insight call the extension once for all
agents; they do not read Digest TOML/history or Core storage. Digest uses public
Core identity/stat operations in batches, never one subprocess per member.

A successful document has `version: 1`, `observedAtMs` (snapshot assembly time),
and `members`, with one row per identity from public Core identity listing,
including members without overrides. Rows are sorted by canonical `identityId`;
an empty inventory is `members: []`. Names are display text, never action selectors.
Each row contains:

| Field                        | Definition                                                                                                                                          |
| ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `identityId`, `name`         | Core canonical identity UUID and current display name.                                                                                              |
| `mode`                       | Resolved `auto`, `interval` or `off`, following [Settings](#settings).                                                                              |
| `intervalMs`                 | Resolved positive duration in milliseconds for interval mode; null otherwise.                                                                       |
| `settingSource`              | `default` for inheritance, `member` for a current override.                                                                                         |
| `settingValue`               | `default` for inheritance; otherwise `auto`, `off` or the current duration, normalized for display/action selection.                                |
| `setByIdentityId`, `setAtMs` | Current override attribution as defined by Settings; both omitted for inheritance, setter ID omitted when unknown.                                  |
| `heldCount`                  | Core held count, or null when unavailable; missing evidence is never zero.                                                                          |
| `statsObservedAtMs`          | Core observation timestamp for the held snapshot, or null when unavailable.                                                                         |
| `nextSendAtMs`               | Configured interval deadline of the oldest held message, or null without a known interval deadline.                                                 |
| `timeToSendMs`               | Zero when the interval flushCount threshold is reached; otherwise `max(0, nextSendAtMs - observedAtMs)`. Null when required inputs are unavailable. |
| `savings`                    | Null throughout phase 2; unavailable savings must not be rendered as zero. Phase 3 defines its estimate before changing this field.                 |
| `labels`                     | Ordered ready-made labels defined below.                                                                                                            |

Timestamps and numeric counts are nonnegative JS-safe integers. Deadline arithmetic
that cannot be represented safely is unknown. `nextSendAtMs` is an extension
deadline, never Core's `nextEligibleAtMs`. It is null for auto/off, zero held
messages, or unavailable oldest-held timing. Phase-2 auto means when idle, not a
predicted timestamp. A deadline or “Due now” label never proves the agent is idle
or authorizes delivery past Core's admission/approval guards. `flushCount` can make
an interval batch due earlier: keep the scheduled `nextSendAtMs`, but report
`timeToSendMs: 0` and “Due now” when its known threshold is reached. Neither
condition establishes Core eligibility. This matches tick scheduling; the renderer
does not infer a threshold from the held count.

Status is read-only: it does not tick, claim/deliver messages, refresh provider
transcripts, collect history or change settings. Failure of identity/settings
discovery fails the command rather than emitting an empty successful inventory.
Unavailable stats keep the known identity/settings row and use null evidence.
Consumers acquire status off their paint path; an absent/slow extension or failed
command leaves its presentation unavailable and never blocks a frame or tab.
Unknown additive JSON fields may be ignored; unsupported versions or malformed
required fields are unavailable, not guessed defaults.

## Labels

Each label is `{text, colorClass, action}`. Text is plain display text (no markup,
ANSI or shell syntax); `colorClass` is a semantic role from the shared
[design tokens](../../../../design/tokens/tokens.json), not a raw color or CSS.
This version uses only `text`, `dim`, `muted` and `waiting`. Consumers render the
supplied text and role; they do not recalculate Digest rules. `action` is null
for a display-only label. There are at most three labels, in this order:

1. Mode/interval: `Auto`, `Every 20s`, `Every 5m`, or `Off`. Auto/interval use
   `text`; off uses `dim`. This chip alone has a choose action.
2. Held count: `N held` with `muted`, only for known N greater than zero.
3. Timing: omit for off; otherwise `No held messages` (`dim`) at zero,
   `Unknown` (`dim`) for unavailable timing/count, `When idle` (`muted`) for
   auto with held messages, `In 20s` (`muted`) for a future interval deadline,
   or `Due now` (`waiting`) when effective `timeToSendMs` is zero. Positive countdowns
   round upward to whole seconds before unit formatting; numeric fields retain milliseconds.

Interval display uses the largest exact unit among d/h/m/s/ms, so 300000 ms is
`5m` and 1500 ms is `1500ms`. Choices are ordered Default, Auto, 1m, 5m, 10m,
30m, 1h, saved custom duration, Off. The default label exposes its resolved value:
`Default (Auto)`, `Default (5m)` or `Default (Off)`; its value is always `default`.
A non-preset current custom duration is included as, for example, `20s (custom)`.
Saved custom values, when available, follow the preset choices without duplicates.
The current choice is `default` for inheritance, including inherited auto; an
explicit auto override selects `auto`. There is no flushCount choice or CLI flag.

The Ops UI owns authorization, mouse/keyboard controls and its final `Custom…` entry with
Set/Cancel input (#2152). That entry is UI, not a synthetic `custom` CLI value.
It submits a duration through the same action; clearing submits `default`.
Ops applies squad defaults per member; Digest has no squad concept or UI lead rule.

## Actions

The supplying extension namespace is `digest`, known by the consumer from the
command it invoked, not trusted from a response field. Two tagged shapes exist:

- Choose: `{kind:"choose", options:[{label,value}], current, argv}`. Option
  labels are plain text, values are strings and `current` names an offered value.
  Settings values/durations follow Settings; no TOML parsing or parallel grammar.
- Run: `{kind:"run", argv}`. This executes a fixed action after explicit user
  activation. Run is supported by the seam, but adds no fourth Digest chip.

`argv` is a nonempty array of strings without NUL, never a shell command. Before execution,
consumers require `argv[0]` to equal the supplying namespace exactly; `tmt`,
`ops`, another extension, an executable path or an unknown action kind is refused.
No fallback execution is permitted. Run has no substitutions. Choose replaces
one whole argv element equal to `{value}` with the selected value; embedded,
missing or repeated placeholders are refused. Other placeholders are unsupported.
Member UUIDs are already literal argv elements, never a `{member}` substitution.

The member setting action is
`["digest","10000000-0000-4000-8000-000000000001","{value}"]`.
The consumer spawns `TMT_EXECUTABLE` directly with the resulting array, preserving
each value as one argument; it never joins the array, invokes a shell, looks up a
response-supplied executable or interprets selected text as syntax. A custom
duration from explicit UI input uses this same substitution and Settings validation.
Failures are shown to the user without claiming that a setting was saved or
automatically repeating a possibly effected command.

The vectors include inherited/explicit auto, inherited interval, custom interval,
off, empty and unavailable held evidence, unknown setter, a due deadline and early flush.
Action vectors freeze literal argv expansion, run, and namespace/shape refusals;
they are consumer examples, never commands to run during fixture verification.
