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
