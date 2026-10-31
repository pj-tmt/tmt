# Browser pages

The standalone `/join` instructions entry belongs to Remote, including its fixed Hosting
resources; it never mounts Colab or a Session. `invite_page` shares the compiled document,
stylesheet, renderer and CSP between the composer and native `Pages`. `join.js` erases the
private fragment synchronously before creating the stylesheet link or importing `invite-v1.js`;
failed erasure loads neither. `invite-page.ts` owns bounded v2 parsing and the current copy
interaction, with no Auth/provider/storage access. Expiry, fragment replacement and page departure
clear its retained token and fence late import/copy feedback. Only explicit Copy produces the
native join command. The [invitation contract](../../../../contracts/remote-channel-v1.md#native-invite-and-join)
owns authority, publication capability, privacy and uncertainty; browser tests cover the actual
CSP, requests, clipboard action, history, disposal and responsive presentation.

The Remote-owned pairing HTML inlines `assets/pair.js` as its first synchronous script under
an exact CSP hash. It captures/erases the fragment before any script import or subresource;
`no-referrer` and `no-store` remain. `Pages` exposes only the public current descriptor through
`/sdk/pair-offer`. [Pairing](../../../../contracts/remote-channel-v1.md#pairing) owns the manual
ceremony, native `#auto=CODE` first-use flow, cancellation proof and local-process boundary.
The automatic flow labels its page “Connect this browser”, hides the pairing eyebrow and uses
“Not connected” for its blocked state; manual pairing retains its own wording.
The automatic form is hidden before paint; the SDK either reuses/cancels with the stored device
or enrolls exactly the read-only default, then replaces the same tab with its validated target.
No code enters logs, requests, ordinary output or continuation links. Used/cancelled/expired
codes show a plain refusal and a machine-side reopen command. Storage and unconfirmed
reopen failures retain the saved pairing, with no pair-again instruction or silent enrollment.
Only a genuinely absent or verified revoked/expired pairing permits enrollment. Commands use code styling and stay unbroken within the pairing status.
A same-document fragment change restarts capture/erasure and cancels the previous page attempt. Browser fixtures cover erasure before SDK loading, first enrollment,
returning-browser cancellation, another profile's replay refusal and code-free Back/Forward.

Manual initialization keeps Pair disabled until the descriptor and submit listener are ready.
Its keyboard flow keeps the named comparison region, explicit word confirmation, retained key,
non-stolen focus and Return to Remote behavior. Failed setup never enables enrollment. Existing
manual/foreign-origin/expiry/proof controls remain, with no extra authentication gate for Firebase.

`Pages` embeds checked `design/browser-ui/generated/static.css` followed by
Remote's `assets/pages.css` with compile-time `concat!(include_str!(...))`.
Cargo and installed serving never run Node or a generator. The shared package owns
browser roles, fonts/header tokens and Header/Notice/Field/Action presentation;
Remote defines host metrics, viewport layout, safe-area offsets and one window
scrollbar. Regeneration and drift checks belong to the
[shared package](../../../../design/browser-ui/README.md). Remote has no copied palette or shadow styling. Host pages use 14px body text,
controls at least 36px on desktop and 44px on narrow/touch screens, with flat, unframed states. Static hosts use the shared aurora SVG from the
leaf’s static-host contract, pinned by Remote-owned literal
tests; Cargo does not read the leaf’s React source or fetch the mark at runtime.

Static markup uses shared class/slot and native accessibility contracts.
`browser.ts` supplies explicit notice tones and visible state words; decorative marks
never establish access. Socket tests verify exact shared-plus-host CSS bytes and
unchanged CSP/types. Browser cases compare computed presentation with current browser
tokens at 1440/390 light/dark and 320 fit, retaining keyboard focus and disabled/loading
states. Standalone notice pages retain their identifying `h1` above the state `h2`. App routes instead put their single `h1` in the shared top bar; loading and unavailable states retain that route identity. Document titles and shell menu/back navigation remain with the shell owner.

The SPA root installs shared top-bar bindings before route adoption. Remote-owned routes put
their single h1 in the bar's left title slot, with module/name/section segments; the shared leaf
hides prefixes and truncates the last segment on phones. Ops and TMT Settings update that title
for each section, including loading and unavailable states. Content keeps its one-line subtitle
and page actions, without a duplicate heading or hero sentence. The shell retains document titles,
Back navigation and common controls; the GUI-owned leaf controls their order. The leaf owns the single trusted Command/Ctrl+K listener; Remote owns the
confined module prefetch, registry and pending-open lifetime. Navigation is unavailable until the
live composed shell describes its admitted modules and sections. A capability-gated
extra shares its extension mount and cache lifetime; extension-wide counts and
the Colab loader use only the primary description. Unconfirmed session counts stay
unconfirmed, rather than using a device count. Search uses Colab's `mountSearchSession` entry;
Colab owns transport admission, the dialog and focus return. Prefetch imports code only after a
live description admits the entry; an unpaired page acquires no extra session or read. Route or
verified Session replacement disposes search and provider registrations, and a late import cannot
open an old route. Root search navigation reuses the management model's existing hold reason for
unsent names and unconfirmed operations. `Search TMT…` names no adapter that has not shipped.
After disposal, a persisted document restoration reloads through the normal entry, preserving
the stored pairing rather than keeping ended sessions or asking to pair again.

## Browser entry

The human serve link names `/`; protocol readiness/status/SDK addresses retain `/r/<prefix>`.
Home and TMT settings follow the [public route contract](../../../../contracts/remote-channel-v1.md#public-browser-routes); they expose only navigation, no state or authority.
`Pages` serves the global list at `/remote/` and the current-door Overview at `/remote/overview` and same-origin `/sdk/landing.js` bootstrap under the existing
CSP/no-store/Host/Origin boundary. `browser.ts` validates local pairing before network, then checks
once on page open through descriptor, verified Session and scope-free `capabilities` observation.
Manual Check again reuses the current Session. An expired Session is silently reopened once
within the manual check; an ended capabilities response is not a pairing refusal. No polling,
inventory, work or grant repair occurs.
Missing/unreadable storage sends nothing. Seven explicit icon-and-text states distinguish local
pairing, checking, connected, different machine, signed refusal and unconfirmed access. Generic
Connected uses no invented friendly name. Technical evidence and local-time checked time stay in
native Details; command Copy actions report denial honestly and leave selectable text.

Only signed refusal supports Not accepted. Its body names pairing removal/expiry only for verified
revoked/expired authority; a session-ended/evicted body offers Check again without claiming pairing
loss. Opaque404 remains unconfirmed even after a
descriptor-only recheck, never another admission. Page-lifetime/signing fences prevent new
requests after departure; per-check fences prevent late painting. A valid stored pairing short-circuits manual enrollment, leaves its offer open and continues
to `/remote/overview`; unreadable/unconfirmed state never enrolls silently. Pair success still links back
to `/remote/overview`, with unchanged fragment erasure, terminal confirmation and disabled-until-ready controls.

The native CLI regression fetches the printed human URL as HTML and verifies protocol-base
refusal with zero request-triggered core calls. Existing Chromium owners cover exact pairing,
entry states, keyboard activation, storage/transport/pin errors, refusal, token metrics and
1440/390 light/dark captures plus a 320-pixel fit check. The SDK fixture covers complete stored-record guards
and page replacement; actual browser/terminal fixtures remain the user-flow evidence owner.

The Remote rail keeps **My Remotes**, this machine's **People** and **My devices**, and a final
**Remote settings** row in the shared settings slot. One `registryView` projection feeds its
known registry total and existing recovery group command; absent member/device counts stay unknown. The rail does
not enumerate a second remote list. This machine is first, explicitly marked, including when
no public reference has been saved. Each row shows its backend, mark plus connection word,
and the viewer role from the existing confirmed readable management observation: a designated
settings browser is Admin; a readable non-designated pairing is Member (M1 browser grants are
users). Unreadable or unconfirmed access is Role unknown. This display grants no authority.
Foreign entries show `◌ Not checked` and are never probed. Current access reuses the page's
existing observation and grants no additional authority. Create a Remote opens the global
`/remote/create` guide; Join a Remote opens the global `/remote/join` guide.

The in-app Join guide uses the same invitation carrier and native command as the standalone
`/join` instructions. It shows local expiry only and gives terminal steps for comparison and
confirmation; it does not enroll, check reuse or certify completion. The code remains in the
current interaction and is cleared on expiry, route departure and pagehide.

The Create guide keeps type/step/project/region inputs in the retained Remote model. It uses the
same optional saved layers/budget observation and refresh, without another client or admission.
This machine needs no cloud account; Firebase steps name the terminal or provider console;
Cloudflare stays disabled as coming soon. Missing prerequisites, plan/permission checks and
deployment completion remain Not checked. Browser-local naming uses the existing registry dialog
and CAS, never a deploy flag. One product command catalog supplies every copy-only reference;
the selected project/region are validated before substitution. Preview is JSON without authorization;
the separate terminal deployment reference leaves plan review and y/N with the person. Copy never
executes either. Guide steps mark visited steps as done without checking prerequisites or completion;
command tokens stay whole and scroll inside their block, and denied-copy feedback clears on success.
Result facts and access-gated Recheck share one card; disabled Recheck explains the missing access.
Service shares one preview for both changes and permissions. A saved enabled layer
or copied command never certifies a completed deployment. Unknown/partial outcomes keep the original
terminal record and its recovery instructions. Guide listeners and late reads follow route disposal.

Create is the primary page-head action; Join contains invitation guidance and remembering an
address. Row titles open their Remote; the shared overflow menu owns Rename and destructive
Forget, while passive chips show role and this-machine context. Add, rename, browser-local
Forget and confirmed collision-aware import follow the
[browser registry contract](../../../../contracts/remote-channel-v1.md#browser-remote-registry).
Forget states that access is unchanged and offers Undo, which re-adds only the public reference
through the same collision/CAS checks. Handoff fragments are erased before admission. Registry
changes touch only `remotes-v1`; the device record and non-extractable handle remain unchanged.
Firebase Open preserves its validated public link and does not send catalog data. Foreign Open
never retargets an existing client. Settings blocks it during a request, unknown original outcome
or unsent device name; the original reason and recovery remain on that page. The list and rail
reuse shipped browser-ui roles/icons; Remote owns only their page composition. No optional
signed catalog/default read or client factory is shipped.

## The one web app and public entries

Every app route (`/`, `/settings`, `/remote/` and its current-door tabs, `/ops`, `/digest`, `/colab/…`)
is the static `assets/shell.html`; `shell-app.ts` routes inside it, adopting each page's markup
from `assets/*.html` and loading Colab's module from the composed shell's `entry`. The pairing
page stays its own document and mounts the rail only.
Remote’s `remote-routes.ts` is the one browser route/menu declaration. My Remotes is the global
origin-local list; its Open enters the selected door’s Overview. One current-door frame owns the
Overview, People, Invites, My devices, Service & upgrade and Settings tabs, with no Remote ID in
the path. The native page allowlist mirrors that explicit set, and old machine/devices/connect
paths have no shims. The [public route contract](../../../../contracts/remote-channel-v1.md#public-browser-routes)
owns the exact paths. `remote-menu.ts` maps Create/Join to My Remotes and Service to Remote settings;
Overview selects the Remote module itself. Rail children stay fixed across connection and route changes.
`remote-app.ts` keeps both markup models and persistent tab panels mounted, moving the same form
nodes instead of cloning or remounting their draft/original-outcome owners. Tabs use the shipped
static roles, roving focus, keyboard selection and disposable resize visibility. A registry
observation subscription supplies role and clears it on access loss; no second source or authority
is created. The verified landing Session initializes management once; unavailable management
initializes read-only without another discovery. Manual connection rechecks do not trigger an
extra management admission; later management reads retain their bounded identity-checked recovery. Member admin-only tabs stay visible with lock word/mark, reason and a copy-only request.
Unknown access has its own text. The existing composed-shell observation supplies Needs-you counts.
`invitation-page.ts` retains the invitation decision model on People under that verified Session; an Invites row awaiting the owner opens it there.
Entry/explicit refresh reads the shared current candidate; it creates no offer or poller.
Comparison and sending start unchecked and are separate choices. An explicit decision
uses the SDK's existing signing lane and one original operation ID. Uncertainty keeps
that ID for read-only lookup across tab/Session changes, disables another decision and
never retries the effect. A proven refusal clears that pending decision and offers
Check invitation; a terminal-winner refresh shows the actual current outcome.
Route departure clears candidate/comparison state and fences
late painting; native designation and Session checks remain the authority owner.
A document-level registry modal remains visible from every tab and is removed on route abort;
late validation cannot write or reopen it.
Invites (`invites-page.ts` over `issued-invitations-page.ts`) reads `remote.invites.list` only
for the connected owner, on entry and after its own effects; members see the locked tab. Invite sends one
`remote.invites.create`; the first response's code and link exist only in its dialog and model memory
and are cleared on close, never stored or re-read. Uncertain creation keeps its operation ID for a
metadata-only lookup. Cancel is offered only for active rows; refusals map to the contract reasons. `/p/<id>` still answers 308 to `/colab/p/<id>`.
Remote forwards only `/read/<id>` in place to Colab's public mount socket, with the
Remote-computed `tmt-mount` header.
For that entry it owns no Colab catalog, capability or CSP; no cookie/session lookup occurs.
The [contract](../../../../contracts/remote-channel-v1.md#transport-binding-loopback-http)
owns grammar, admission and reply policy. Real-door tests pin exact extension paths,
header isolation, generic unavailable refusals and unchanged SDK mapping. Browser seam
fixtures assert no Remote redirect, unchanged short URLs on reload and fragment secrecy;
Colab's integrated app-routing acceptance remains Colab-owned.

## Owner reads in the app

`owner-reads.ts` is the one client for the [owner browser reads](../../../../contracts/remote-channel-v1.md#owner-browser-reads):
`ownerRead(name, signal)` reads `<mount>/x/-/<name>` (the mount from the shell's `shellAddress`)
and checks the versioned envelope, so a page gets `ok` with the payload, `unavailable` with a
contract reason (an unknown one is `unknown`), `no-session` (no owner mount, or no pairing to open
one) or `unreachable` (no read, no network). The door's generic 404 means its owner session ended,
as on a door restart: the read reopens it from the browser's pairing once and reads again. Reads
that meet the same restart, such as Digest's digest and limits, share that one reopen; it stops
only when every waiting read's signal aborted. `whileVisible(intervalMs, signal, tick)` refreshes only while the route is mounted and
the document visible, at once on showing again, doubling the wait after each failed read up to
eight times. `/ops` reads `ops` every 5 s through `ops-source.ts`, repaints only when the board
changed and gives each reason one sentence with what to do; Digest uses the same client.
`test/owner-reads.test.ts` runs every vector of `owner-reads-v1.json` through it.

`ops-menu.ts` projects fixed children, observed totals and Agents-state attention into the shared
rail, with Ops settings last. Missing, unavailable, truncated and null evidence produces no pill.
`ops-menu.ts` also supplies this machine's squad channels and member DMs through the same
shared menu: one channel per squad with member children keyed by canonical UUIDs retained by `ops-source.ts`.
No name-to-identity resolution, extra board read or product-local rail is added. The route retains squad expansion across observations and navigation; the shared leaf owns
nesting, disclosure, keyboard traversal and the deepest visible selection on desktop and phone.
`ops-channel.ts` renders local channel Members and DM About panels with four plain states
(Working, Idle, Unknown, Offline) from explicit presence/activity, not attention or raw board
colour. Both panels label timers as squad timers; an owner display name never associates one
to a member UUID. Failed timer reads and partial boards retain unavailable/partial wording.
DM history is explicitly unavailable with a terminal reference; unread is not shown. The
browser never discovers request IDs or treats the Remote journal as conversation history.
Sending and timer/idle-reminder writes remain disabled with their reason. Existing workspace
sections remain reachable. Unchanged observations retain the selected tab in its always-open panel;
removed members clear their old facts rather than selecting a namesake.

`ops-page.ts` retains section selection and the read lifetime; `ops-agents.ts`,
`ops-squads.ts` and `ops-timers.ts` render the observed board through shared
`ops-parts.ts` presentation helpers. Ops settings (`ops-settings.ts`) has no browser read: it
says where the settings are, with the terminal card (`terminalCard`, also Timers' way to manage
them) and links to the Agents and Squads views. Squads shows member cards and delegates its
read-only detail dialog to `ops-drawer.ts`. A refresh updates an open detail view;
an absent agent is reported without retaining stale facts. Dismissal resolves the
current member button after refresh, with the Squads heading as its focus fallback.
Leaving the section or aborting the route closes the dialog. These views add no
agent or schedule controls, owner reads or persistence.

## Settings page

The current-door management tabs use the same embedded shared CSS, Header/Notice/Field/Action classes and
Remote host metrics; its additional stylesheet owns settings layout and static listbox geometry.
Normal settings use thin section rules, a desktop summary/control grid and a single-column
narrow layout. Each setting keeps its unchanged hint and original-result feedback beside its
Save action; loading triggers show no invented default value.
Button-triggered, named listboxes use native top-layer popovers with shared static menu CSS:
arrow/Home/End/typeahead navigation, Enter/Space choice, Escape/Tab dismissal and trigger focus
return. Choice changes stay local until Save; the display-only unset Default stays disabled.
The [settings-page contract](../../../../contracts/remote-channel-v1.md#remote-settings-page) owns second-press
consequences, draft retention and navigation guards. `settings-controls` supplies presentation;
no operation ID exists before a deliberate confirmation against the admitted target.
The existing page and CSP owners remain unchanged. Remote's `management-page` owns admitted values,
frozen intent/outcome and separate current-access state; `settings-controls` owns the two static
listboxes, while `settings-page` binds forms and inline consequences without
resetting unsent text during async failure/read refresh. Refresh retains the current device
page; navigation focuses and refuses to leave an unsent device name until saved or restored,
so only one bounded page of forms is retained. The separately generated
`settings-v1.js` imports the single served public SDK, without another key/channel owner.
Both generated modules must pass rebuild byte equality. The owning
[management protocol](../../../../contracts/remote-channel-v1.md#remote-settings-page)
defines the unlimited default/source, capacity, durable last use and atomic remove-others behavior.
Settings combines opening/session controls, Name in this browser (catalog-only CAS), and the honestly labelled current browser/device name. Remote-wide and person names have no new setter. My devices filters the same admitted page to this browser; a missing row is reported, never inferred. One state-aware pairing guidance owner replaces the repeated status/Connect blocks. Only the locally designated browser may remove devices; read-only pages show its exact designation
command. Removed rows disappear without deleting revocation tombstones.

The existing Chromium pairing fixture also exercises the actual settings page: designated/
read-only values, exact custom cap/off/default source, role removal, retained drafts/caret,
explicit sending-scope enable/disable, lost self-change acknowledgments, original-only recovery,
no resend and process cleanup. Sending controls preserve device-name drafts and other grant policy.
The single frozen original projects into one pre-mounted empty polite status slot beside its setting
or device actions; an absent row falls back above the forms. Recovery stays beside the unknown
result and reads only that original once. Its row omits a duplicate disabled reason; other rows
say “Another change is still unconfirmed. Check its original result first.” Original IDs appear only for
unknown or refused outcomes. Capacity uses “Browser change limit reached. Use the local CLI; do not
retry or reset storage.” Unchanged settings Saves stay disabled with an associated
no-change hint;
draft comparison retains unset/default provenance, and submit-time guards allocate no operation.

A >25-device scenario covers later-page drafts through refresh, another committed effect,
original-receipt recovery and guarded first/next navigation.
Use `TMT_REMOTE_CAPTURE_DIR` for light/dark 1440/390 settings captures; 320 fit is asserted.
Captures include loading/read-only/disabled reasons, drafts/focus, outcome/recovery and self-change
states at 1440/390 light/dark and 320 fit. Refresh restores its initiating keyboard control
only when the person has not moved elsewhere. Device forms and action names use the admitted
device name, never an unsent draft. The capture index records the served state and provenance.

Service & upgrade is read-only: it explicitly requests the signed settings projection,
sharing status's saved evidence and dated published limits. Backend and add-on availability
come from that observation; unsupported add-ons remain unavailable, including paid file
storage until it ships. Cloudflare is coming soon. Missing deployed/required version, plan
and permission observations stay **Not checked**. Copy-only references use `status --layers`
and a `deploy firestore --defaults --json` preview without `--authorize`, from the single
`assets/service-commands.json` catalog shared with the native parser proof; they run only when
the person copies them into this machine's terminal. Provider-console links explain where
to review project setup, permissions and billing. Refresh values rereads this machine's
saved facts after the person acts; it is not a live provider check. No browser execute or
confirmation action, provider call, native field or second readiness owner is added. The page
never displays reason codes, measured usage or an invented quota warning; ready sharing includes the
[contract's fixed limits note](../../../../contracts/remote-channel-v1.md#local-cli-discovery). Native Details
contains the wrapping budget table and its date, Pacific reset and local guard thresholds.
An optional observation failure clears stale evidence without overwriting management access,
original outcome or drafts. Fixtures cover every prerequisite; capture labels distinguish
injected tier/quota from private recorded evidence and never claim provider acceptance.

Unchanged access and Firestore projections do not repaint on unrelated draft input. Short polite
slots announce only changed settled states; the layer lists, budget and Details stay outside
those live regions. Presentation equality never suppresses signed reads or authority checks.

## TMT Settings General

General and the Ops terminal cards share `commandCopy(text)`: a terminal reference, a Copy button and inline success/manual-copy feedback, with no command execution.

General uses the shared shell module menu and page inset. Its appearance previews keep the existing
browser-only System/Light/Dark theme owner and keyboard choice. Core configuration remains read-only:
observed source rows are shown as supplied; without a browser read, the amber card names
`tmt config show` and its Copy control reports success or manual-copy guidance. Copy never executes
commands. No reference defaults or sample records are substituted for machine values. Specific source
refusals remain visible, and sample injection stays labelled. The route signal still fences late reads.
The machine-guidance card links to the existing Modules section; it does not introduce a source read.

## TMT Settings Identities

Identities uses the shared Settings shell menu and inset, with a terminal-reference notice and
read-only inventory. Only source-supplied rows produce the Name/Lifetime/Provider/Presence table
and local selected detail. Rows retain live-first then name order; saved/temporary lifetime is
separate from active/offline presence. Codex and Claude use their product labels; a missing
provider says Not reported. The public table selection frame and aria-current mark the chosen
row; selecting it adds no read, mutation, route or persistence. Long names wrap and the detail
stacks below the table on phone. Without inventory the page says unavailable or empty, preserves
specific refusal text, and names tmt ls with the shared nonexecuting Copy. Injected samples remain
labelled in the notice and inventory; the duplicated sample line is hidden on this section.
Source failure and route-disposal fences remain with the existing page/source owner. A source-read
inventory is a neutral read-only note; only a missing read or refusal is the amber warning, and the
name button's inset focus frame has padding around the name.

## TMT Settings Modules

Modules uses the same shell menu and inset: a subtitle, a read-only note with `tmt extension ls` and the
shared nonexecuting Copy, and one card of module records (glyph tile, name, one line of product copy,
an Installed or Not installed state with its mark, the channel and version in mono). The page keeps
one static map of copy for the product's own modules (`tmt-settings-copy.ts`); another module shows
no line. An installed module gets a Settings action only from the composed shell's description of it,
which can arrive after the read and repaints the card; an uninstalled one never has an action. The
Remote card and its link appear only for an installed, described Remote. Without a browser read the
page shows the amber notice and the command and no record, version or channel; sample injection stays
labelled "Sample installed modules". No install, update or read is added by this page.

## TMT Settings Drivers & hooks

Drivers & hooks takes the same inset and shell menu: a subtitle, a note, two nonexecuting Copy commands
(`tmt driver ls` and `tmt setup --status`), a Host drivers card, an Agent hooks card and a card that
says what each role is. A driver is its name, the product's line for herdr (the one map in
`tmt-settings-copy.ts`; another name shows no line), an Approved state and the version in mono; a
driver that no longer runs as approved shows its reason in the waiting tone instead of Approved. A hook
is its provider, whether usage collection is on, and a Set up state. Without a browser read the page
shows the amber notice, the commands and the roles card and no record, version or state; sample
injection is labelled "Sample data". The page installs, approves and removes nothing.

## Digest presentation

`digest-page` combines per-seat activity with Token usage through its Stacked/Lines and Absolute/Share
views, reusing the same reported buckets and palette. Missing observations break lines rather than
becoming zero. A browser-local Name/Usage view preference sorts the seat table and both axes of the
full-width Messages heatmap. Invalid or unreadable stored preferences initially fall back to Usage;
later choices remain active for the mounted page even when persistence fails. Storage holds no report data.
Messages retains all observed participants, quiet zeros and diagonal, and a focusable internal scroller
for large teams. Its owner-local table uses existing accent/text tokens for the sequential tint.
Token areas use the leaf's pastel seat palette and monotone interpolation; sampled values and
incomplete-bucket notes remain unchanged. `digest-page` derives the allowance cards, combined token chart and seat legend,
Messages heatmap and coverage/method footer from `DigestSource`. Allowance uses a fixed
`digest/7d` owner read independently of the selected usage range (the 7d selection shares that
read); `limits` supplies history. Each mounted source reuses that one allowance result
across range switches (including in-flight reads). A switch reads only its selected usage
report; 7d reuses the weekly report itself. The minute refresh obtains a fresh allowance
result. There is no global cache. Limit titles come only from the explicit source `provider`,
never seat model families. Active weekly windows group by provider, duration and reset; the
newest observation reported by a current seat leads each provider, with other windows folded
and labeled by reset, reporter seats and observation age. Histories match that exact window.
A provider with two or more accounts (Core's optional `account {key,label}`, #5274) gets one card per
account, titled such as `Claude · work` (today Core names accounts only for Claude, from its allowance
source), and the windows that name none are `unknown account`; one
account, or none, is one card as before. The key only groups and is never shown, the label is plain
text, and a window or history entry with a malformed account is dropped, never folded into another's.
A provider with no active window is an explicit not-reported card (for Claude: Claude Code does not report
its allowance to TMT); a Claude window from Core's allowance source is shown like any other. Provider history and method
stay disclosures. Provider cards use a horizontal snap carousel with equal heights, a next-card peek
on phones and previous/next controls when it overflows. The page owns range, chart style, scale,
seat order, expanded legend and disclosure state; changed
reports restore open disclosures and summary focus, while unchanged reads retain the existing
DOM and scroll position. Failed refreshes retain the last good report for that range; an
unavailable newly selected range does not display the preceding range's observations. The
route signal owns refresh cleanup. The page uses existing owner reads and does not change allowance accounting or persistence.
