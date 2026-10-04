# Board internals

## Home

`board::home` retains a board-only summary, shared-filter attention sections and
squad tile model as typed `View.home: Option<home::Home>`; other views
carry no home data.
It reuses `tab_view` acquisition and the user-tab section pipeline. Its optional
observed ages come from the existing staleness observer: the home tab starts
one for every squad before its roster read and records afterward, writing its
observation cache under the held per-squad lock when enabled and available.
It respects the reminders policy without extra core commands. Request ages
use shared-inbox timestamps; pending-only rows have no age. The source aggregate
document and `ls --tab all` JSON/text remain unchanged. The home painter uses
one summary band and body, bypassing ordinary pane composition for the shown
immutable home view. `home::tiles` supplies pure lines and local item/line/x/width
regions from one admitted Taffy grid: three columns at 150 cells, two at 100,
and compact rows below 100 or with at least ten visible squads (two compact
columns at 150). The controller's filtered reading order stays row-major.
Each tile shows its squad, lead/model/window totals/share and exclusive non-lead
urgency marks/member count; compact rows retain the last two lead windows.
Whole-roster summary and attention counts retain their existing semantics.
Unknown member states contribute to the member count without inventing a mark.
Runtime `App::home_usage` owns observations, model attribution and the configured
longest-window share; tiles only format them. Unavailable and measured zero
remain distinct; partial readings and shares carry `~`. Uniform window labels
appear once in ③; mixed configurations use a “windows vary” legend and label
each tile's totals and displayed share with its actual window.

Home keeps one `App.selected` cursor, reconciled by section/squad/member identity
across refresh and search. Attention precedes squads; future replies and cron
targets insert between them. Home translates tile regions into global ordinals,
complete selected-range reveal and viewport-clipped continuation hits through
one `Scrolls` pass. Selection covers every padded tile row; gaps and headings
have no hit target. Ordinary pane geometry and scalar rows retain their owners.
Enter jumps to a member or opens a squad; Tab traverses attention/squads, and `a`
opens the real request picker or an annotation to the selected squad’s lead.
The composer retains and revalidates sender, target, lead and open request before
public `tmt answer` or annotation dispatch. Questions stay inside the picker;
tiles show no member names, task/PR fields or private question text. Replies feed
and cron acquisition/lifecycle remain separate owners. A future cron summary
supplies an explicit stable clock-key target, not a squad target.
