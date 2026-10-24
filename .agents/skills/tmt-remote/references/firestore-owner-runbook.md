# Set up Firestore for Remote

Follow these steps on the machine that will own the deployment. Replace the placeholders
with your own inputs; keep the deployment record on that machine.

## 1. Prepare your project and tools

Start with TMT, Remote and Colab installed. Remote reads the enabled extensions' installed
deployment declarations; it does not substitute sample configuration when one is unavailable.

The shared [deployment input contract](../../../../contracts/remote-channel-v1.md#deployment-input-manifest)
defines explicit resource selection and `--defaults`. Supply every input explicitly, or
use `--defaults` for omitted choices. Missing inputs refuse before login; replace any
`<...>` placeholders in the printed command before rerunning.

Choose an existing Firebase project on the free Spark plan. TMT does not create the project
or set up paid features. Choose the database region carefully: an existing database's
location cannot change, and an authorized deployment keeps its recorded region.

Install Node.js and the supported Firebase CLI, then sign in with your own owner account:

```sh
npm install -g firebase-tools@15.29.0
firebase login
```

In the Firebase console, open Authentication and choose **Get started** once. This guide
uses the default `anonymous` sign-in selection. If you explicitly choose
`google.com` instead, enable Google under Authentication → Sign-in method when instructed.
Signing in to the CLI, pairing a browser or installing an extension never authorizes deployment.

## 2. Review the changes and answer y/N

Run this yourself in an interactive terminal:

```sh
tmt remote deploy firestore --project <project-id> --region <region> --defaults
```

`--defaults` selects database `(default)`, a new Web App named `tmt Remote (<deployment>)`,
a dedicated TMT Hosting site and anonymous sign-in. It does not supply project or region.
Only `(default)` is supported on the free plan in this version. To use existing resources,
supply `--database '(default)' --web-app <app-id> --site <site-id> --sign-in anonymous`;
`--web-app new` and `--site new` explicitly request the creation choices. Existing IDs must
belong to the selected project. New sites do not overwrite its project-default site.

Check the account, project, region, Security rules and Website files in the short summary.
For a hosted deployment, also review `Register this machine as owner`. This explicit step reuses
this machine's anonymous sign-in and registers its native public identity with your deploy login;
it is part of your approval, before Rules publication. Anonymous sign-in alone permits no native
agent-talk write. [The contract](../../../../contracts/remote-channel-v1.md#native-invite-and-join)
owns invitation and peer access; deployment never admits a person to send messages.
Add `--details` to see the full plan, fingerprints, extension resources and indexes before
the question. Security rules include the native agent-talk namespace and its displayed bounds,
plus the enabled extensions' policies; deploying them does not connect or admit a peer.
A target shown without live change evidence still needs your review; a saved completed run is not proof that the current project is unchanged.

```text
Deploy these changes to your Firebase project? [y/N]
```

Answer `y` only after reviewing the changes, including any replacement of rules or website
files set outside TMT. Your answer approves the captured inputs, not a newly discovered
configuration. No or end-of-input exits successfully without changing your Firebase project
in that invocation. Earlier changes from a partial run are not undone.

This command has no `--yes` or `--no-open` flag. For a read-only preview, use:

```sh
tmt remote deploy firestore --project <project-id> --region <region> --defaults --json
```

A preview still reads Firebase using your login. Agents, CI, non-interactive input and
`--json` do not approve changes implicitly. To deploy through those paths, review the full
plan and rerun with `--authorize <plan-digest-prefix>`: at least the first 12 lowercase
hexadecimal characters of that plan's digest, with the same inputs. `--details` does not
change the plan or grant authorization.

## 3. Let it finish, or resume the same deploy

Remote records each outcome before proceeding. It creates pending composite indexes together,
then checks them for up to three minutes with visible elapsed-time progress. If an index is
still building, follow the printed instruction to rerun the same command in a minute.

After a partial result or an instructed console action, rerun the same command with the same
inputs. Omitted choices reuse the local deployment record, including its actual app/site and
older defaults. Explicit conflicting resource choices refuse before provider setup; do not
delete the record to move a deployment. Legacy records keep their recorded project-default site. An identical, unfinished deploy you already approved continues without another y/N
question. A completed run asks again. Read the stop table below before changing anything.

Keep the saved `deploy.json` at the path printed by the command. A partial or unknown result
may include applied changes; it is not a rollback or a usable joint publication. An unknown
creation is observed before further work and is never blindly created again. If the same
unconfirmed stop persists, keep its diagnostics for a fix rather than deleting the record
or repeatedly retrying.

If the captured inputs changed, review the new summary. The terminal asks once before
replacing an unfinished deploy and keeps its earlier outcomes; nothing is undone. When the
earlier website's publication has not been attempted, the displayed warning can permit leaving an
uploaded or possibly created website untouched and not live. This does not bypass an unknown
web-app, site or site-configuration creation. Non-interactive `--authorize` retains the
stricter supersession fences; follow the printed instruction if replacement is refused.

## 4. Check the recorded result

In an interactive terminal, one live status line shows the current work, upload count and
elapsed time after five seconds. It clears before the final result. Final verification can
take a few minutes; JSON and non-interactive output do not add this live line.

Look for `Run: complete; usable binding: true` and the emitted `Remote link:`. Complete
requires the deployment's joint read-back; pending or unknown work is not success. The link
contains public connection configuration, not an access grant. To print it again:

```sh
tmt remote link --project <project-id> --json
```

Open that link for the [anonymous connect guide](../../../../contracts/remote-channel-v1.md#anonymous-connect-guide).
It observes this browser's sign-in and offers commands to add/sign in on a machine, without
claiming those commands ran. Give a reader the Colab page link separately: it admits the browser
with the link's role. Opening it does not join the machine's native sync identity.

For local browser management, use the [door's Remote settings entry](../../../../contracts/remote-channel-v1.md#public-browser-routes).

To inspect the locally recorded evidence:

```sh
tmt remote status --project <project-id> --layers --json
```

Status makes no provider call. Its meaning and limitations belong to the
[status-layer contract](../../../../contracts/remote-channel-v1.md#local-cli-discovery);
it does not establish project-wide quota headroom or prove a guest can use the deployment.

## If a step stops

Use the printed code and outcome, not a guessed provider response. An early read failure's
“Nothing changed” describes that invocation, not a rollback of earlier runs. Diagnostic
fields and lifecycle rules belong to the [deployment contract](../../../../contracts/remote-channel-v1.md#authorized-deploy).

| Stop or printed code                                                                                                            | What to do                                                                                                                                               |
| ------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `REMOTE_DEPLOY_INPUT_REQUIRED`, `REMOTE_DEPLOY_INPUT_INVALID`                                                                   | Supply the listed inputs, replace placeholders, and review the defaults and creation list.                                                               |
| `USAGE_ERROR`, `REMOTE_DEPLOY_PROJECT_INVALID`, `REMOTE_DEPLOY_LOCATION_INVALID`, `REMOTE_DEPLOY_SIGN_IN_MISSING`               | Correct the project, region or explicit sign-in input before approving again.                                                                            |
| `REMOTE_DEPLOY_TOOL_MISSING`, `REMOTE_DEPLOY_NODE_MISSING`, `REMOTE_DEPLOY_TOOL_UNSUPPORTED`, `REMOTE_DEPLOY_SETUP_UNAVAILABLE` | Follow the printed tool or login instruction using the supported CLI and your own account.                                                               |
| No enabled Firestore declaration, `REMOTE_DEPLOY_DECLARATION_UNAVAILABLE`, `REMOTE_DEPLOY_HOSTING_UNAVAILABLE`                  | Check that the enabled installed extensions and Remote support the requested deployment.                                                                 |
| `REMOTE_DEPLOY_OWNER_INITIALIZE_AUTH`, `REMOTE_DEPLOY_OWNER_ENABLE_GOOGLE_SIGN_IN`                                              | Perform the named Authentication console action, then rerun the same command.                                                                            |
| `REMOTE_DEPLOY_BUSY`                                                                                                            | Let the active deployment finish; do not remove its lock or record.                                                                                      |
| Target-selection or local-state refusal                                                                                         | Follow the printed targeting or state diagnostic while preserving the deployment history.                                                                |
| `REMOTE_DEPLOY_PROJECT_CONFLICT`, `REMOTE_DEPLOY_DATABASE_MISMATCH`                                                             | Use the correct recorded target and existing database region/edition; do not reset the record to bypass the mismatch.                                    |
| `REMOTE_DEPLOY_ACCOUNT_CHANGED`, `REMOTE_DEPLOY_ACCOUNT_UNREADABLE`                                                             | Resolve the owner-login evidence before approving or resuming.                                                                                           |
| `REMOTE_DEPLOY_AUTHORIZATION_STALE`                                                                                             | Read the current preview and review its digest before authorizing; preserve the earlier outcomes.                                                        |
| `REMOTE_DEPLOY_PREVIOUS_CREATE_UNCONFIRMED`, `REMOTE_DEPLOY_PREVIOUS_VERSION_STAGED`                                            | Keep the record and follow the printed original-outcome or permitted-supersession instruction.                                                           |
| `REMOTE_DEPLOY_PERMISSION_DENIED`, `REMOTE_DEPLOY_API_DISABLED`                                                                 | Resolve the named access or API prerequisite before resuming.                                                                                            |
| `REMOTE_DEPLOY_QUOTA_EXCEEDED`                                                                                                  | Check the relevant Firebase quota and resolve it before resuming; TMT does not measure remaining project quota.                                          |
| `REMOTE_DEPLOY_PROVIDER_REJECTED`                                                                                               | Use the printed operation and HTTP status for diagnosis; an arbitrary 400/404 is not a reason to wait and blindly retry.                                 |
| `REMOTE_DEPLOY_OWNER_CONFLICT`                                                                                                  | Restore the original machine identity or correct the selected project; Remote will not overwrite another native key or inviter binding. Keep the record. |
| `REMOTE_DEPLOY_RULES_FOREIGN`, `REMOTE_DEPLOY_VERIFY_FAILED`                                                                    | Review the captured replacement or read-back evidence before further approval; keep the record.                                                          |
| `building`                                                                                                                      | Follow the printed provisioning or index-wait instruction and rerun the same command.                                                                    |
| `unknown`, unconfirmed uploads or `REMOTE_DEPLOY_UNAVAILABLE`                                                                   | Read any printed `Cause:` and `File:` samples and the saved outcome, if present, then use the same command to observe the original.                      |
| `REMOTE_LINK_UNAVAILABLE`                                                                                                       | Finish the verified deployment before expecting a remote link.                                                                                           |

## If a fix requires an update

Keep the record and the captured extension inputs while a deploy is unfinished. For an
approved Remote helper-only fix, update Remote alone:

```sh
tmt extension upgrade remote --yes --json
```

Then resume the same command. A general `tmt upgrade` can also change Colab's captured
content and therefore the approval digest. If those inputs change, use the review and
supersession flow above; do not delete the record to bypass an unconfirmed creation.
Keep diagnostic contents private: do not post project, account or provider resource names.
