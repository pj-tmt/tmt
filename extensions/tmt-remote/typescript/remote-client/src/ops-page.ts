/** The Ops web view, read-only: Agents, Squads, Timers and Ops settings, one section at a time from
 * the module menu. Every state is a mark and a word; what the reader sets on a section stays across
 * the board's refreshes. */
import { pageLoading } from './page-loading.js';
import type { RemoteShell } from './remote-shell.js';
import { notAvailable } from './not-available.js';
import { agentsView } from './ops-agents.js';
import { settingsView } from './ops-settings.js';
import { squadsView } from './ops-squads.js';
import { timersView } from './ops-timers.js';
import { opsSource, type OpsBoard, type OpsSource } from './ops-source.js';
import { whileVisible } from './owner-reads.js';
import { channelView } from './ops-channel.js';
import { OPS_SECTIONS, opsMenu, opsPlace, type OpsSection, type OpsPlace } from './ops-menu.js';

/** The board's own reload cadence (`tmt ops ui`). */
const REFRESH_MS = 5000;

export function opsPage(shell: RemoteShell, source: OpsSource = opsSource): void {
  const main = document.getElementById('ops-section');
  const note = document.getElementById('ops-note');
  if (!main || !note) return;
  const href = (id: OpsSection) => `${location.pathname}#${id}`;
  const current = (): OpsPlace => opsPlace(location.hash);
  let board: OpsBoard | undefined;
  // Each section keeps what the reader set (filter, search, collapsed squads, selection) across
  // repaints and across leaving and coming back.
  const views = {
    agents: agentsView(),
    squads: squadsView(),
    schedules: timersView(),
  };
  let settings: HTMLElement | undefined;
  const channel = channelView(location.pathname);
  const paint = () => {
    const section = current();
    if (section !== 'squads') views.squads.close();
    shell.setMenu(opsMenu(section, board, location.pathname));
    const label =
      typeof section === 'string'
        ? OPS_SECTIONS.find((item) => item.id === section)!.label
        : section.memberId
          ? (board?.agents.find(
              (agent) => agent.id === section.memberId && agent.squad === section.squad,
            )?.name ?? 'Member unavailable')
          : `#${section.squad}`;
    const chips: string[] = [];
    if (board && !board.unavailable && typeof section !== 'string') {
      const squad = board.squads.find((item) => item.name === section.squad);
      const timers = board.schedules?.filter((item) => item.squad === section.squad);
      chips.push(
        squad
          ? squad.idleAlertMinutes
            ? `Idle alert ${squad.idleAlertMinutes} min`
            : 'Idle alert not configured'
          : 'Idle alert unavailable',
        timers && !board.truncated ? `Timers ${timers.length}` : 'Timers unavailable',
        'Read only',
      );
    }
    shell.setTitle(label, ['Ops'], 'Ops', chips);
    if (!board) main.replaceChildren(pageLoading());
    else if (board.unavailable) main.replaceChildren();
    else if (typeof section !== 'string') {
      if (channel.node.parentElement !== main) main.replaceChildren(channel.node);
      channel.update(board, section);
    } else if (section === 'settings') {
      // The board's settings have no browser read: the page says where they are.
      settings ??= settingsView(href);
      if (settings.parentElement !== main) main.replaceChildren(settings);
    } else {
      const view = views[section];
      if (view.node.parentElement !== main) main.replaceChildren(view.node);
      view.update(board);
    }
  };
  window.addEventListener('hashchange', paint, { signal: shell.signal });
  shell.signal.addEventListener('abort', () => views.squads.close(), { once: true });
  paint();
  // The board while the route is visible, at the board's own cadence; a page repaints only when
  // the board changed, so a reader's place and selection stay.
  let last = '';
  whileVisible(REFRESH_MS, shell.signal, async () => {
    const next = await source.board(shell.signal);
    if (shell.signal.aborted) return true;
    const seen = JSON.stringify(next);
    if (seen !== last) {
      last = seen;
      board = next;
      if (next.unavailable) {
        notAvailable(note, next.unavailable);
      } else {
        if (next.truncated)
          notAvailable(
            note,
            'The board is larger than the browser shows. `tmt ops ui` shows all of it.',
          );
        else {
          note.textContent = '';
          note.hidden = true;
        }
      }
      paint();
    }
    return next.unavailable === undefined;
  });
}
