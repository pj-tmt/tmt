/** The Ops web view, read-only: Agents, Squads, Timers and Ops settings, one section at a time from
 * the module menu. Every state is a mark and a word; what the reader sets on a section stays across
 * the board's refreshes. */
import { pageLoading } from './page-loading.js';
import type { RemoteShell } from './remote-shell.js';
import { notAvailable, notInBrowser } from './not-available.js';
import { agentsView } from './ops-agents.js';
import { sentence } from './ops-parts.js';
import { squadsView } from './ops-squads.js';
import { timersView } from './ops-timers.js';
import { opsSource, type OpsBoard, type OpsSource } from './ops-source.js';
import { whileVisible } from './owner-reads.js';

/** The board's own reload cadence (`tmt ops ui`). */
const REFRESH_MS = 5000;

const SECTIONS = [
  { id: 'agents', label: 'Agents' },
  { id: 'squads', label: 'Squads' },
  { id: 'schedules', label: 'Timers' },
  { id: 'settings', label: 'Ops settings' },
] as const;
type Section = (typeof SECTIONS)[number]['id'];

export function opsPage(shell: RemoteShell, source: OpsSource = opsSource): void {
  const main = document.getElementById('ops-section');
  const title = document.querySelector<HTMLElement>('.tmt-ui-title');
  const note = document.getElementById('ops-note');
  if (!main || !note) return;
  const href = (id: Section) => `${location.pathname}#${id}`;
  const current = (): Section =>
    SECTIONS.find((section) => `#${section.id}` === location.hash)?.id ?? 'agents';
  let board: OpsBoard | undefined;
  // Each section keeps what the reader set (filter, search, collapsed squads, selection) across
  // repaints and across leaving and coming back.
  const views = {
    agents: agentsView(),
    squads: squadsView(),
    schedules: timersView(),
  };
  const paint = () => {
    const section = current();
    if (section !== 'agents') views.agents.close();
    if (section !== 'squads') views.squads.close();
    shell.setMenu({
      title: 'Ops',
      groups: [
        {
          items: SECTIONS.filter((item) => item.id !== 'settings').map((item) => ({
            id: item.id,
            label: item.label,
            href: href(item.id),
            current: item.id === section,
          })),
        },
        {
          items: [
            {
              id: 'settings',
              label: 'Ops settings',
              href: href('settings'),
              current: section === 'settings',
            },
          ],
        },
      ],
    });
    if (title) title.textContent = SECTIONS.find((item) => item.id === section)!.label;
    if (!board) main.replaceChildren(pageLoading());
    else if (section === 'settings')
      // The board's settings have no browser read: the page says where they are.
      main.replaceChildren(sentence(notInBrowser('The Ops settings page', 'tmt ops ui')));
    else {
      const view = views[section];
      if (view.node.parentElement !== main) main.replaceChildren(view.node);
      view.update(board);
    }
  };
  window.addEventListener('hashchange', paint, { signal: shell.signal });
  const closeDrawers = () => {
    views.agents.close();
    views.squads.close();
  };
  shell.signal.addEventListener('abort', closeDrawers, { once: true });
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
      if (next.unavailable) {
        closeDrawers();
        board = undefined;
        main.replaceChildren();
        notAvailable(note, next.unavailable);
      } else {
        board = next;
        if (next.truncated)
          notAvailable(
            note,
            'The board is larger than the browser shows. `tmt ops ui` shows all of it.',
          );
        else {
          note.textContent = '';
          note.hidden = true;
        }
        paint();
      }
    }
    return next.unavailable === undefined;
  });
}
