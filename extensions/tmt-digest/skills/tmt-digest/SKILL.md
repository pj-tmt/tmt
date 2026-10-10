---
name: tmt-digest
description: Configure a member's digest mode and run the minute tick that applies it through Core.
---

# Digest

Digest holds a member's automatic messages and delivers them together. Core still decides when the member can receive them.

## Set a member's mode

```bash
tmt digest worker 5m
tmt digest worker auto
tmt digest worker off
tmt digest worker default
```

MEMBER is a saved identity name or UUID. A duration such as `30s`, `5m` or `1h` saves an interval. `auto` and `off` both deliver messages normally for now; only an interval holds messages. `default` removes the member setting and inherits the global default; explicit `auto` remains a member override.

The global default is `auto`. Global `default` and `flushCount` are configured in `digest.toml` beside the global configuration file shown by `tmt config show --json`. The shipped flush count is 10; it must be a positive integer and applies only to interval mode.

```toml
default = "auto"
flushCount = 10
```

Member commands save configuration. They do not start a timer or immediately change Core's delivery policy. The next tick applies them.

## Run the minute tick

```bash
tmt digest tick
```

When the Ops board clock is running, it runs the tick for you. Without Ops, run `tmt digest tick` once a minute from cron or launchd (child timeout 90 s). Each invocation stays until the next minute boundary; another invocation exits quietly while the first holds the lock. A Core call already in progress may finish within its existing bound before tick exits.

For interval mode, tick holds automatic messages and delivers a digest when the oldest has waited the interval, or sooner when the flush count is reached. A due message still needs Core's live readiness checks; a busy or approval-blocked member can wait longer. Core may still deliver at the end of the member's turn.

Ticks renew interval holds. If ticks stop, the hold lapses after the greater of the configured interval and two minutes, and normal delivery resumes at the next admitted opportunity. Switching to `auto` or `off` clears the hold when a tick applies the setting.

Successful ticks print nothing. Invalid settings, unavailable Core and delivery errors produce a nonzero exit. Do not replay a delivery after an uncertain outcome; Core retains its claim and settlement state.

## Read status

`tmt digest status --json` returns one read-only document for all saved identities, with resolved modes, held-message counts and ready-made labels. Unavailable stats stay unknown; savings are unavailable in phase 2. It does not tick, deliver messages or change settings.

## Inspect help and the skill

Use `tmt digest --help`, `tmt digest tick --help`, `tmt digest status --help` and `tmt digest skill`. The skill command prints the bundled instructions without Core discovery or storage access.

Install the official extension through `tmt extension install digest --skills`. Ask the user before installing. Use the managed extension upgrade path, `tmt extension upgrade digest`; do not overwrite a command managed by another installer.
