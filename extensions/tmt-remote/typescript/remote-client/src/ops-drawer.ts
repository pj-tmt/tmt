/** The agent's details in a drawer: what the squads page opens for a member. Read-only. */
import { browserUiClasses } from '@tmt/browser-ui/static';
import { createIcon } from '@tmt/browser-ui/glyph';
import type { OpsAgent } from './ops-source.js';
import { el, keyOf, readOnlyBadge, stateChip } from './ops-parts.js';

export function agentDrawer(focus: {
  returnFocus(agent: OpsAgent): HTMLElement | undefined;
  fallback(): HTMLElement;
}): {
  node: HTMLDialogElement;
  open(agent: OpsAgent): void;
  update(agents: readonly OpsAgent[]): void;
  close(): void;
} {
  const dialog = el('dialog', browserUiClasses.drawer);
  dialog.setAttribute('aria-labelledby', 'ops-drawer-title');
  const title = el('h2');
  title.id = 'ops-drawer-title';
  const close = el('button', `${browserUiClasses.action} ${browserUiClasses.iconActionControl}`);
  close.type = 'button';
  close.dataset.variant = 'text';
  const closeIcon = el('span', browserUiClasses.iconActionIcon);
  closeIcon.append(createIcon('x'));
  close.append(closeIcon);
  const closeHost = el('span', browserUiClasses.iconAction);
  closeHost.append(close);
  const header = el('header', browserUiClasses.drawerHeader);
  header.append(title, closeHost);
  const body = el('div', browserUiClasses.drawerBody);
  dialog.append(header, body);
  let selected: OpsAgent | undefined;
  close.addEventListener('click', () => dialog.close());
  // A press on the backdrop (the dialog element itself, outside its box) dismisses it.
  dialog.addEventListener('click', (event) => {
    if (event.target !== dialog) return;
    const box = dialog.getBoundingClientRect();
    const inside =
      event.clientX >= box.left &&
      event.clientX <= box.right &&
      event.clientY >= box.top &&
      event.clientY <= box.bottom;
    if (!inside) dialog.close();
  });
  dialog.addEventListener('close', () => {
    const agent = selected;
    selected = undefined;
    if (!agent || !dialog.isConnected) return;
    // Refresh replaces member buttons. Resolve the current row instead of retaining a dead opener.
    const target = focus.returnFocus(agent);
    target?.focus({ preventScroll: true });
    if (!target || document.activeElement !== target)
      focus.fallback().focus({ preventScroll: true });
  });
  function render(agent: OpsAgent | undefined): void {
    if (!agent) {
      body.replaceChildren(el('p', undefined, 'This agent is no longer on the board.'));
      return;
    }
    selected = agent;
    title.textContent = agent.name;
    close.setAttribute('aria-label', `Close ${agent.name}`);
    const top = el('div', 'ops-drawer-top');
    top.append(stateChip(agent.state, agent.stateText || undefined), readOnlyBadge());
    const facts = el('dl', browserUiClasses.facts);
    for (const [term, value] of [
      ['Squad', agent.squad],
      ['Role', agent.role === 'lead' ? 'Lead' : 'Member'],
      ['Model', agent.model || '—'],
    ] as const)
      facts.append(el('dt', undefined, term), el('dd', undefined, value));
    const block = (label: string, value: string, tone?: string) => {
      const part = el('div', 'ops-drawer-block');
      const name = el('p', 'ops-detail-label', label);
      if (tone) name.dataset.tone = tone;
      part.append(name, el('p', undefined, value));
      return part;
    };
    body.replaceChildren(
      top,
      facts,
      block('Task', agent.task || 'No task recorded'),
      block('Next step', agent.next || 'No next step recorded'),
      ...(agent.pending ? [block('Waiting on you', agent.pending, 'waiting')] : []),
      el('p', 'ops-detail-note', 'Read-only observation. This panel does not control an agent.'),
    );
  }
  return {
    node: dialog,
    open(agent) {
      render(agent);
      if (!dialog.open) dialog.showModal();
    },
    update(agents) {
      const key = selected && keyOf(selected);
      if (dialog.open && key) render(agents.find((agent) => keyOf(agent) === key));
    },
    close() {
      // Departure is navigation, so it does not return focus to a row in the view being removed.
      selected = undefined;
      if (dialog.open) dialog.close();
    },
  };
}
