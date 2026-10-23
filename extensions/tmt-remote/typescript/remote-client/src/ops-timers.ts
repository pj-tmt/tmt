/** The timers page: the schedules as the board records them, and how to manage them. */
import { browserUiClasses } from '@tmt/browser-ui/static';
import { createIcon } from '@tmt/browser-ui/glyph';
import type { OpsBoard } from './ops-source.js';
import { el, pageFoot, pageHead, readOnlyBadge, sentence } from './ops-parts.js';

/** The one command that shows the timers, with a Copy button that says what happened. */
function managePanel(): HTMLElement {
  const card = el('section', browserUiClasses.card);
  card.setAttribute('aria-labelledby', 'ops-manage-title');
  const tile = el('span', browserUiClasses.glyphTile);
  tile.append(createIcon('terminal'));
  const title = el('h3', browserUiClasses.cardTitle, 'Manage in the terminal');
  title.id = 'ops-manage-title';
  const text = el(
    'p',
    undefined,
    'Use the Ops workspace to manage your timers. This web view does not change schedules.',
  );
  const command = el('span', browserUiClasses.command);
  const copy = el('button', browserUiClasses.commandCopy);
  copy.type = 'button';
  copy.setAttribute('aria-label', 'Copy tmt ops ui');
  copy.append(createIcon('copy'), 'Copy');
  const feedback = el('span', browserUiClasses.commandFeedback);
  feedback.setAttribute('role', 'status');
  copy.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText('tmt ops ui');
      feedback.textContent = 'Copied.';
    } catch {
      feedback.textContent = 'Copy failed. Select the command and copy it manually.';
    }
  });
  command.append(el('code', browserUiClasses.commandText, 'tmt ops ui'), copy, feedback);
  card.append(tile, title, text, command);
  return card;
}

export function timersView(): { node: HTMLElement; update(board: OpsBoard): void } {
  const page = el('div', 'ops-timers-page');
  const grid = el('div', 'ops-grid');
  const card = el('section', browserUiClasses.card);
  card.setAttribute('aria-labelledby', 'ops-timers-title');
  const head = el('div', browserUiClasses.cardHead);
  const title = el('h3', browserUiClasses.cardTitle, 'Timers');
  title.id = 'ops-timers-title';
  head.append(title, readOnlyBadge());
  const body = el('div');
  card.append(head, body);
  grid.append(card, managePanel());
  page.append(
    pageHead(
      'Keep the rhythm visible.',
      'A simple view of the schedules that keep your team moving.',
    ),
    grid,
    pageFoot(),
  );
  return {
    node: page,
    update(board) {
      if (board.schedules === null)
        return body.replaceChildren(
          sentence('Your machine could not read the timers. Run `tmt ops ui` to see them.'),
        );
      if (!board.schedules.length) return body.replaceChildren(sentence('No timers are set.'));
      const scroll = el('div', browserUiClasses.tableScroll);
      const table = el('table', browserUiClasses.table);
      table.dataset.roomy = '';
      const headRow = el('tr');
      for (const text of ['Timer name', 'Squad', 'Schedule', 'Owner', 'Timer ID']) {
        const cell = el('th', undefined, text);
        cell.scope = 'col';
        if (text === 'Owner' || text === 'Timer ID') cell.dataset.compactHide = '';
        headRow.append(cell);
      }
      const thead = el('thead');
      thead.append(headRow);
      const rows = el('tbody');
      for (const timer of board.schedules) {
        const name = el('td', undefined, timer.what || timer.id);
        // Under the compact width the owner and the id move under the name.
        name.append(
          el(
            'span',
            browserUiClasses.tableCompactMeta,
            [timer.owner, timer.id].filter(Boolean).join(' · '),
          ),
        );
        const when = el('td', browserUiClasses.tableMono, timer.when);
        const owner = el('td', undefined, timer.owner || '—');
        const id = el(
          'td',
          `${browserUiClasses.tableMono} ${browserUiClasses.tableQuiet}`,
          timer.id,
        );
        for (const cell of [owner, id]) cell.dataset.compactHide = '';
        const tr = el('tr');
        tr.append(name, el('td', undefined, timer.squad), when, owner, id);
        rows.append(tr);
      }
      table.append(thead, rows);
      scroll.append(table);
      body.replaceChildren(scroll, el('p', 'ops-note', 'Schedules are shown as recorded.'));
    },
  };
}
