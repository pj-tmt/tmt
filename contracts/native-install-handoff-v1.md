# Native CLI installer handoff, version 1

This internal protocol joins the running managed CLI updater to the verified
candidate CLI. It is not an extension API, a public offline installer, or a way
to authenticate another process of the same user. The implementation owner is
`tmt-adapters::native_install::handoff`; the typed CLI grammar admits its version.
[Architecture](../ARCHITECTURE.md#managed-skills-and-native-installation) owns the
trust boundary and publication lifecycle.

PR acquisition and returning from a PR channel use separately probed protocol 2,
defined in [the PR channel contract](native-pr-channel.md). Protocol 1's published
shape and normal release behavior remain unchanged.

Protocols 3 and 4 are protocols 1 and 2 for an updater that validates the CLI prefix
`lib/tmt` instead of `lib/tmux-team`; see
[the install architecture](../.agents/skills/tmt-core-runtime/references/install-architecture.md#cli-prefix-move).
Their probe, bounds, request fields, result and failure rules are those of protocol 1
(protocol 3) and protocol 2 (protocol 4), with `protocol` set to the version and, in
the PR wrapper, the inner `transfer.protocol` set to `3`. A candidate serves protocols 1 to 4: for
1 and 2 it keeps publishing under `lib/tmux-team` while the CLI is active there, so an
updater released before the move never sees a path it does not validate. Updaters send 3 and 4 only.

## Admission and probe

The parent verifies immutable GitHub release metadata, product/tag/target match,
manifest and archive digests, compressed/expanded bounds and every archive entry
before executing any candidate code. It stages the verified tree privately.
Executing the candidate has the same trust as installing that verified release.
Archive checksums detect changed bytes, not a compromised release origin.

The parent invokes the staged executable with:

```sh
tmt __native-install --handoff-version 1 --probe --json
```

The probe has no installation input or mutation. Success is exit 0, empty stderr
and the JSON object `{"protocol":1}` with no additional fields. A probe deadline
is ten seconds; each output stream is bounded to 16 KiB. A failed, unsupported or
malformed probe prevents installation. Public upgrade reports
`NATIVE_UPGRADE_INSTALLER_UNSUPPORTED` with this message:

```text
This release needs a newer installer: rerun install.sh with: curl -fsSL https://github.com/pj-tmt/tmt/releases/latest/download/install.sh | sh
```

Already-published binaries without this handoff retain their original errors.
The diagnostic cannot retroactively change them. Their recovery is a bootstrap
reinstall, preserving the intended prefix.

## Installation request

After a successful probe the parent invokes the same staged executable with
`__native-install --handoff-version 1 --json`, sending one JSON object on stdin.
Input is bounded to 16 KiB with a five-second read deadline. Unknown fields fail.
The command cannot combine handoff mode with offline installation arguments.

| Field                               | Value                                                             |
| ----------------------------------- | ----------------------------------------------------------------- |
| `protocol`                          | Integer `1`                                                       |
| `archive`, `manifest`               | Absolute paths to the private staged original files               |
| `prefix`                            | Managed installation prefix observed by the parent                |
| `target`                            | Verified native target, also required to match the candidate host |
| `version`                           | Exact selected release version                                    |
| `archive_sha256`, `manifest_sha256` | Verified SHA-256 digests                                          |
| `channel`                           | `stable` or `alpha`                                               |
| `pin`                               | `preserve`, `pin`, or `clear`                                     |
| `expected_current`                  | Observed current receipt UUID as a string                         |
| `release_id`                        | Positive GitHub release ID                                        |

The candidate checks the manifest digest, verifies the archive using its own
strict inventory and confirms the selected version/digest. Under the existing
installation lock it rechecks the expected receipt, ownership, target, version
and pin policy. It uses the existing publisher and records GitHub provenance;
there is no network refetch, recursive handoff, inventory exception or second
publisher. The parent holds no installation lock while the child runs.

## Result and failure

The installation child has a 60-second deadline and a 16 KiB per-stream output
bound. A completed result has empty stderr and one JSON object with exactly:

- `protocol`: integer `1`.
- `installation`: null or an object with `executable`, `active_executable`,
  `version` and Boolean `changed`.
- `error`: null or a diagnostic string.

Success requires exit 0, an installation object and no error. Failure requires
exit 1 and an error; an installation object is included only when activation
completed before a finalization failure. The parent validates the selected
version, managed command path and immutable release path, without interpreting
the candidate's receipt inventory. An unsupported probe never reaches this phase.

Pre-activation failure retains the old receipt/current release and bytes.
Post-activation failure remains an explicit partial result. A timeout, lost or
malformed result after installation starts is uncertain: the parent instructs
inspection before retrying and never claims rollback from missing evidence.
The parent checks cancellation before probing and before requesting installation.
Once the installation child starts, that bounded child owns its transaction;
parent interruption is reported after it settles, not as proof of rollback.
The bounded process runner owns child termination/reaping; the invocation owns
its staging tree. Provider skills are refreshed by the newly active executable
using `__native-refresh-skills --managed --json` under its own installation lock, and that executable then owns the consented
extension phase. Neither phase is an application-wide atomic transaction.

## Consented extension progress

The newly active CLI retains extension inventory and publication authority. The parent
first invokes `__native-upgrade-extensions --json --plan --progress`, with no apply input.
Exit 0 requires the existing plan JSON on stdout and exactly
`{"upgradeProgress":1}` followed by a newline on stderr. A clean exit 2 permits one
read-only legacy `--plan` call, without `--progress`; other failures refuse the phase.
The plan must match the products, installed versions, pin states and exact targets shown
before approval. The legacy plan/result JSON shapes and 64 KiB stdout/input bounds remain.

After consent, a capable CLI receives the same plan on stdin with `--yes --progress`.
An older CLI receives `--yes` without progress. Apply runs once, never as a capability
probe or a retry after missing/malformed output. The phase retains its 180-second bound;
a pending Remote adds only the existing Remote-owned restart allowance. The existing
process owner multiplexes input/stdout/stderr and owns deadline failure, termination and
reaping; the progress receiver does not start or supervise another process.

Progress is optional observation on stderr: newline-delimited objects with exactly
`version` (integer 1), `product` (an official extension name), and `event`. Each line is
at most 1 KiB, and the stream at most 1 MiB. Event objects have a `kind`:
`selected` adds `version` and `archive_bytes`; `download` adds `received_bytes` and
`total_bytes` (integer or null); `settled` adds Boolean `warning`; `downloaded`,
`verifying`, `verified`, `activating`, `activated` and `failed` have no other fields.
Counts come from actual response bytes; Actions acquisition observes the payload ZIP,
not the catalog ZIP. Download observations are throttled; the last actual byte count is
flushed before the next phase. A missing HTTP total retains the selected release size.
Unknown fields, versions, products, malformed or incomplete lines invalidate
the progress report; they never authorize a second apply or prove rollback. Final
activation/skills results still come only from the validated stdout document and exit.
Failure to render or emit observations cannot veto a verified publication.
