# Ops UI configuration migration

Approved specification for #2921; implementation is in progress. This describes the direct replacement with Overview, Agents and Digest. There is no legacy UI or legacy-default preset. Compatible explicit bindings override the new defaults; unsupported actions/scopes/settings are reported and require deliberate migration. User files are preserved until a confirmed, format-preserving write.

### Current defaults and key consumers

| Existing production behavior | Proposed successor/default |
|---|---|
| Enter/double-click = host jump in tmux, menu in plain terminal; aggregate Enter = squad tab | Enter selects/activates the workspace target; `j` explicitly goes to host when available, Menu is an action. Single click selects. Old explicit `jump` remains host-specific. |
| Up/Down and j/k navigate; PageUp/Down/Home/End scroll/select | Arrows and paging retain navigation. Overview/Agents `j` becomes Host; reader/picker j/k remain navigation. No global j/k interception ahead of scoped bindings. |
| Left/Right change old squad tabs; Shift+Left/Right reorder them | Three shell tabs selected by guarded 1/2/3; remove old per-squad-tab reorder. Located migration notices explicitly name old Left/Right and Shift+Left/Right behavior; no invisible fallback retains it. Squad sorting/pinning is the workspace successor. |
| `s` conditional switcher fallback | Overview `s` cycles sort and `S` reverses it. Old switcher/default retired; an explicit `s` binding wins if its action remains compatible. |
| `t` Talk; `o` Open; `c` conditional cron list | Keep `t` Talk and `o` Link; `c` Timers. In list scope add `x` Remove membership, `m` Add/reassign, `p` Pin, `n` Create squad. Member rows retain `D` Digest. Member Rename is deferred to Core #2986; WP2 has no member `r` binding or hint. `w` is reserved for WP5 snapshots and cannot be rebound to another list action in WP2. Reader and cron p/x/o/d retain their local meanings. |
| `a` annotate lead; `A` ask-lead on member boards or home-write on aggregate; `e` home-message; `v` view-reply; `@` home-pick | Remove implicit aggregate/lead-target defaults. Select the named lead/member and use WP1 Talk/conversation; annotation, reply and recipient choice remain explicit actions. No automatic migration to a different recipient. |
| `i` Digest; `n` Notes; `y` Copy; `d` conditional detail/replies fold; Backspace = back | Digest tab 3 and member `D` Digest; `n` now Create squad. Notes/Copy remain actions with compatible custom binds. Backspace retains non-editor back where applicable. Remove the old default fold action; new layout preference replaces manual factory-pane folding. |
| Tab = next pane; `/` search; `?` help; `,` settings; Ctrl-R refresh; q/Esc/Ctrl-C quit/back | Keep scoped focus traversal/search/help/settings/refresh and close/quit semantics; inputs/modal owners run first. Preserve Ctrl-R override behavior and explicit refresh bindings. No shell digits fire in editors, search, completion, picker or popup. |

Source correction: there is **no hard-coded L, A or f branch** in the inspected production App key handler. L/A also appear in configured test fixtures; A is nevertheless a real context-dependent preset. Set lead succeeds as the squad action, Ask lead as Talk on the resolved lead row. `with_action_keys` t/r/w/T/l is test-only, not shipped defaults. The workspace parser admits scoped navigation and action keys; compatible explicit bindings override the defaults through the actual user/host/section table. Reader/picker navigation remains local.

Overlay-local contracts remain scoped: reader Esc/q/v close, s source selection, y copy, a annotate; notebook scroll and link traversal; cron n/e/E/p/x/o/d/v and Enter owner; settings text/Enter/Esc/Ctrl-U and reset; help Esc/?/q; form Tab/BackTab/Enter/Ctrl-N/Shift-Enter/Ctrl-U; Picker query/list handling; WP1 completion gets arrows/Enter/Tab/Esc first. Validate scope collisions against these real consumers, not an empty replacement table. The bottom footer is the single key legend; content buttons use action names, except the accepted inline link hint.

### All 25 configurable actions

| Actions inventoried | Disposition |
|---|---|
| `jump`, `back`, `open`, `copy`, `notes`, `refresh`, `token-window`, `theme`, `settings`, `run`, `next-pane`, `menu`, `talk`, `view-reply`, `reply`, `annotate`, `digest` | Retain existing effects and exact arguments where the new target/scope supports them. Validate availability; never invent a target for an unavailable action. `view-reply` opens the existing conversation reader. |
| `pick-tab`, `view` | Retire named-tab/factory-view bindings. Bind `overview`, `agents` or `digest-dashboard` for shell selection; choose Auto/Side/Stack through workspace Settings. No old view argument is guessed into a layout. |
| `toggle`, `tab`, `ask-lead`, `home-message`, `home-write`, `home-pick` | Retire old pane/tab/HOME-specific semantics; report replacement: layout preference, Select squad, Talk on named lead/member, or explicit recipient/conversation selection as applicable. No implicit recipient remapping. |

### Settings/configuration inventory

| Current families | Proposed treatment |
|---|---|
| Six factory views `members/team/focus/notes/detail/wide`; board mode/direction/panes/split sizes/collapsed/fold_below and custom split trees | Replace shell composition with the new workspace and WP1 auto/side/stack preference. Remove legacy view UI/default presets. Explicit old compositions get a migration explanation and user choice, not guessed geometry conversion. |
| `tabs` @all/@leads/@tab:name; order/pin/hide/colors and per-tab binding tables | Replace board navigation with three tabs and UUID-based squad pin/sort preferences. Unambiguous explicit squad preferences can be migrated on confirmation; aggregate/custom-query tab settings need located explanations. Preserve existing CLI listing/projection contracts where independently used; do not retain a second legacy board renderer. |
| `layout` crew/team/pr-queue/minimal, rows/columns/hidden columns | Retain notebook state semantics/data and compatible member field definitions; retire their old factory **shell** arrangement. Nonrepresentable custom display fields/columns must be reported, not silently discarded. |
| notes render; state order/rank/color; reminders enabled/stale_after; field-provider configuration | Preserve notebook/domain policy and acquired data; no new history/notebook owner. |
| refresh; token-rate enabled/every/window/reduced_motion and token windows; theme base/roles | Preserve compatible user/squad overrides, known/no-data semantics and reduced motion. Layout/sort/pin add per-user Config preferences only. |
| AskLead prompt policy; global/host/squad/tab/section binds | Retired AskLead-only policy gets an explicit notice; do not inject it into ordinary Talk. Preserve compatible action strings at their corresponding real scope and report obsolete scope/action arguments. Validate the complete merged table. |
| Custom sections (titles/filters/ordering), row templates and section-specific binds | Preserve independently used CLI document/filter semantics; map supported workspace ordering deliberately. Unsupported board-only section composition gets a located migration notice, not a second shell. |
| Deprecated `board.home_replies` | Keep it retired with a notice pointing to the existing conversation/detail action; update the obsolete e-key hint. |
| opener/clipboard programs, link handlers, labels.sources, me/me_id identity and tmux hotkey configuration | Preserve existing public behavior and consent/selection boundaries. No provider config or host tmux configuration edits. |
