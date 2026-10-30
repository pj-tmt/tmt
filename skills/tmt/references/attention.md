# TMT reference: Exchange attention (X)

Part of the tmt skill, printed by `tmt learn --skill tmt attention`.
The rules of the skill entry (`tmt learn --skill`) still apply.

`tmt x flow --since 24h --json` reads content-free counts and UTF-8 byte totals
for the whole local team in UTC 30-minute buckets, without `--identity`. The
window defaults to 24 hours and accepts integer seconds or ms/s/m/h/d, up to 30
days; `--bucket 7d` (a multiple of 30m) sums rows into larger buckets, one row per
pair and kind when it is as long as the window. Human output sums by identity pair
and kind. No message text, receipts or
request IDs are included; result reads and effective acknowledgments have zero
bytes. Recording begins with the flow migration, independently of body retention.

Bare `x` means `x ls`: unacknowledged retained metadata only, without loading
prompt or final bodies. `--limit` defaults to 50 (1-200); `--after` defaults to 0.
Follow non-null `nextAfter` with `--after`; this is a live revision cursor, not
a frozen snapshot. Deduplicate by request ID; restart at 0 to refresh.

List/show never acknowledge. Single `ack` requires the exact current revision
from ls/show; a stale revision returns `X_REVISION_CONFLICT` (exit 5).
`ackall` needs no prior list, token or batching: it acknowledges the identity's
current write-transaction snapshot and returns `acknowledgedThrough`, not a count.
It does not claim you read every result. A new request or first final committed
after that snapshot remains unacknowledged. Each repeat takes a new snapshot.

Delivery and final are independent. `messageState` says where the message is
(`batched`, `delivered`, `working`, `answered`, `unconfirmed`, `not_sent`); `delivery` is the
raw attempt status and stays `queued` after a Digest checklist was delivered.
`not_submitted` does not mean a task is running; a final may be `retained`, `expired` or `unavailable`. Show exposes exact
retained prompt `message` and final `response`. Acknowledgment neither cancels
work nor deletes content nor asserts success. Settled means a final was submitted
and its current revision acknowledged, even if its body later expires.

Unknown, anonymous, wrong-originator and metadata-expired X records return
`X_NOT_FOUND` (exit 3). Missing caller identity returns `IDENTITY_REQUIRED` (exit 1).
Reads and acknowledgments never renew retention. This is not an offline recipient
queue, memory search or remote access for these originator-facing forms.

Recipient requests use a separate participant attention scope (`--incoming` on
`x show|ack|ackall`, and `x listen`). The focused installed `tmt-inbox` skill owns
the complete processing loop.

Plain `talk` to an identity with no pane queues for its inbox: it accepts one
existing non-retired identity, reports `queued`, and never attempts live
notification or falls back to tmux delivery. A sender has no option to make talk
to a bound identity silent. The recipient must pull with `tmt inbox`; coming
online or enrolling a channel does not deliver the backlog. Human queue output
gives the recipient pull and correlated inspection commands; JSON adds
`notification:"not_attempted"` and `waitingFor:"recipient_inbox_pull"`,
preserving exit 0 and the request ID.
Listen returns a trailing-edge debounced
unread batch, with a 15-minute hard deadline and 10-second quiet default. An idle
deadline is successful `reason:"timeout"`. Listening/showing never acknowledges,
and recipient acknowledgment cannot consume originator response attention. Full
request text and the correlated reply receipt appear in `x show --incoming`.
Announcements use the same inbox with `kind:"announcement"` and
`finalStatus:"not_required"`: inspect and acknowledge them, but do not reply.
Their detail has no reply receipt; the `tmt-inbox` skill owns this processing rule.
