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

## Batch updates for a lead

Use a cadence agreed for the work: for example, `5m` for short routine batches or `20m` while a lead reviews a larger change. `tmt digest lead 20m` saves a recurring interval; it does not end a focus session after twenty minutes. Run the minute tick as described below. When the work ends, `tmt digest lead off` clears the interval at the next tick. `default` restores the global default, which may itself be an interval. `tmt focus lead` shows the lead's pane.

Ordinary automatic sends held by Digest return queued with remaining time. Preserve the request ID and wait for the recipient's reply. Remaining time describes the current hold window, not a delivery promise; a turn boundary or the flush threshold can offer delivery sooner, while readiness and approval can delay it. The recorded owner and `--urgent` bypass Digest only. Keep routine updates in the batch; use urgency for a time-sensitive intervention that cannot wait for the batch. No urgency flag bypasses channel, readiness or approval checks.

The recipient receives one checklist with the original reply commands and receipt for each eligible request, plus result-inspection commands for completed replies. Use those commands for each item; reading or receiving the checklist does not acknowledge X. Follow a printed read/cursor for overflow. After an uncertain handoff, inspect Core's state and retain the original request IDs; Core owns settlement and the checklist must not be replayed.

## Run the minute tick

```bash
tmt digest tick
```

When the Ops board clock is running, it runs the tick for you. Without Ops, run `tmt digest tick` once a minute from cron or launchd (child timeout 90 s). Each invocation stays until the next minute boundary; another invocation exits quietly while the first holds the lock. A Core call already in progress may finish within its existing bound before tick exits.

For interval mode, tick holds automatic messages and delivers a digest when the oldest has waited the interval, or sooner when the flush count is reached. A due message still needs Core's live readiness checks; a busy or approval-blocked member can wait longer. Core may still deliver at the end of the member's turn.

Ticks renew interval holds. If ticks stop, the hold lapses after the greater of the configured interval and two minutes, and normal delivery resumes at the next admitted opportunity. Switching to `auto` or `off` clears the hold when a tick applies the setting.

Successful ticks print nothing. Invalid settings, unavailable Core and delivery errors produce a nonzero exit. Do not replay a delivery after an uncertain outcome; Core retains its claim and settlement state.

## Read status

`tmt digest status --json` returns one read-only document for all saved identities, with resolved modes, held-message counts and ready-made labels. Use `--window 24h|7d|lifetime` (default 24h) for estimated context-input tokens avoided. Lifetime starts when Digest begins collecting; unavailable or incomplete evidence is never zero. This is a gross estimate, not billed tokens or turns saved. Additive `cost` quotes the latest evidenced model at standard base uncached input list price; label it as an estimate at list price, never money saved. `percent` matches whole closed-minute input evidence for 24h or 7d; lifetime percent is null. Missing prices, models or coverage remain null. It does not tick, deliver messages or change settings. Prices come from one public file that `status` refetches anonymously from the repository (2 s deadline, at most every 30 s, no token) and falls back to a private cache and then the built-in table; `pricing.origin` says which. Set `TMT_DIGEST_PRICING_URL=off` to turn that request off.

## Read usage

`tmt digest decisions --json [--since 24h|7d|30d]` lists the decisions each tick recorded: for every member holding messages, the rule that decided it (`interval_deadline`, `flush_count`, `auto_idle_point`, `off` or `not_due`), the held count and oldest held age it saw, and Core's answer to the delivery opportunity (`outcome`, null for `not_due`). Rows hold counts and times only, never message text, and one is written when a member's rule, answer or oldest held message changes; it holds until that member's next row. It reads only Digest's own history and does not tick or deliver. `tmt digest decisions --report --json` reports the turns the delivered digests saved: messages, the baseline of one delivery per message, actual deliveries, `turnsSaved` (an upper bound: a busy agent may combine queued messages anyway) and the delay from arrival to delivery, per member and in total, with what could not be measured counted in `excluded`. `--recompute 20m` adds the same figures for another interval, estimated from arrival times only (Core's readiness wait is not modelled, so its delay is a lower bound; it compares rules and does not predict them). `auto` and `off` cannot be recomputed.

`tmt digest usage report [--since 24h|7d|30d] --json` reads token usage per member and model, account limits with their pace, and who messaged whom. `--csv usage`, `--csv credits` or `--csv exchange` prints the same rows as one table. For Codex, `credits` is the account credit balance per bucket with the service tier seats used (hourly for 24h, daily for 7d and 30d); an absent tier is unknown, not `default`. Everything comes from Core and nothing is stored. Unavailable evidence is listed as unavailable, never as zero, with Core's `cause` when it knows why nothing is sampled (`no_binding`, `no_agent`, `no_sampler`, `not_sampled`: an offline identity, a pane with no agent session, an agent launched outside `tmt run`/`resume`, or one launched there but not sampled, for which relaunching through `tmt run`/`resume` or installing the usage hook helps); a partial seat says so in `coverage`, whose `startsAtMs` is where its evidence begins (Core records from the first read and never backfills earlier time). Claude account limits are unavailable (`no_source`) because Core has no source for them. Windows end at the last completed five-minute boundary (`window.toMs`). Treat names in CSV as untrusted text: a leading `=`, `+`, `-` or `@` is already escaped with `'`.

## Inspect help and the skill

Use `tmt digest --help`, `tmt digest tick --help`, `tmt digest status --help`, `tmt digest decisions --help`, `tmt digest usage report --help` and `tmt digest skill`. The skill command prints the bundled instructions without Core discovery or storage access.

Install the official extension through `tmt extension install digest --skills`. Ask the user before installing. Use the managed extension upgrade path, `tmt extension upgrade digest`; do not overwrite a command managed by another installer.
