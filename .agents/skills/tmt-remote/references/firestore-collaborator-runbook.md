# Talk between two paired machines

Public `tmt talk --remote` sends a prepared message through this machine's running Remote
owner to an already joined peer. It does not start serve, sign in or pair implicitly. A lost
or invalid owner answer stays unconfirmed; keep the request ID and never resend automatically.
The [activation contract](../../../../contracts/remote-channel-v1.md#public-native-agent-activation)
owns this handoff. One-way public delivery is qualified separately from the composed exact final
in originating `tmt result` (#4540); the owner's real-account acceptance remains #4539.

## Install and prepare

On each machine, install the native CLI using the [repository installer](../../../../README.md),
then install Remote:

```sh
tmt extension install remote
tmt --version
tmt remote --help
```

On the deployment owner's machine, follow the [Firestore owner runbook](firestore-owner-runbook.md)
to review and deploy the selected project's Rules and sign-in configuration.
The collaborator needs no Firebase CLI owner login or deployment credential.
Anonymous sign-in alone grants no agent-talk write access; enrollment needs the owner's private invite.

Start the local door on both machines and keep it running:

```sh
tmt remote serve --no-open
```

Use another terminal for the following commands. The serving process owns message
delivery and observes local replies; stopping it postpones delivery without cancelling requests.

## Pair once

The deployment owner runs:

```sh
tmt remote invite --remote <owned-remote>
```

Share the one-use code privately. On the collaborator's machine:

```sh
tmt remote join <code>
```

Compare the four words over a call or a trusted chat. The owner confirms the match,
then chooses whether the collaborator may send. After confirmation, the collaborator
chooses whether the owner may send to their agents. Both sending choices default to no.
This single ceremony sets up both mailboxes; a reply uses the same connection.

Keep the remote reference printed on each terminal. Sending can be enabled later by
the person who owns the receiving machine:

```sh
tmt remote devices
tmt remote devices talk <person-or-exact-client-id> on
```

An ambiguous person name needs the exact client ID. Each receiver controls its own
agent allowlist, hold policy and revoke; permission on one machine grants nothing on the other.

## Send and read replies in either direction

Run each receiving agent under a saved local identity. If you do not already have
one, start it in its own terminal with your installed agent command:

```sh
tmt run --save <local-agent> <agent-command>
```

Use that existing identity as the sender. On either machine:

```sh
tmt talk --identity <local-agent> --remote <peer-reference> <peer-agent> 'Please review this change.'
```

Keep the returned request ID. The receiving agent answers with the ordinary local
receipt-bound `tmt reply` instruction supplied to it. With both serves running, the
encrypted final travels to the originating machine. There, read it locally:

```sh
tmt result <request-id> --json
```

Reverse the roles to talk the other way. After the final is stored, result reads
work offline. Pending or held work has no final yet; it is not permission to send again.

## If delivery stops

Keep the original request ID and frozen message. Never resend automatically after
a timeout or uncertain outcome. Check the receiving machine's local hold/allowlist,
revocation and Remote status; the receiver alone can approve held work. A quota
refusal waits for the guard's next permitted window, not another dispatch attempt.

Upgrade through the ordinary native updater and restart serve when needed. Keep the
existing private state: keys, grants, routes and request receipts survive restart.
Compatible upgrades need no code, pairing or manual reconnect. An incompatible peer
gets the named upgrade refusal; preserve the pairing and follow its upgrade command.
Oversized finals are refused rather than truncated or split into attachments.
