/** Read-only settings presentation; all evidence was validated on the signed SDK lane. */
import { budget } from 'remote-browser-sdk';
import { firestoreTitles, firestoreItems, firestoreReasons } from './firestore.js';
import type { FirestoreSettingsView } from './management.js';
const states = {
  enabled: ['✓', 'Enabled', 'working'],
  'not-enabled': ['—', 'Not enabled', 'review'],
  unknown: ['○', 'Not checked', 'waiting'],
} as const;
function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = ''): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  element.textContent = text;
  return element;
}
function command(text: string): HTMLElement {
  const code = node('code', text);
  code.className = 'tmt-ui-code';
  return code;
}
function mark(state: keyof typeof states): HTMLElement {
  const [icon, text, tone] = states[state];
  const label = node('span');
  label.dataset.tone = tone;
  label.className = 'firestore-state';
  const symbol = node('span', icon);
  symbol.setAttribute('aria-hidden', 'true');
  label.append(symbol, document.createTextNode(` ${text}`));
  return label;
}
function allowance(amount: number, bytes: boolean, period: string): string {
  let written = amount.toLocaleString('en-US');
  if (bytes) {
    const unit = [
      [1024 ** 3, 'GiB'],
      [1024 ** 2, 'MiB'],
      [1024, 'KiB'],
    ] as const;
    const matched = unit.find(([size]) => amount % size === 0);
    written = matched
      ? `${(amount / matched[0]).toLocaleString('en-US')} ${matched[1]}`
      : `${written} bytes`;
  }
  return `${written}${period ? ` per ${period}` : ''}`;
}
export function renderFirestore(
  target: HTMLElement,
  access: 'checking' | 'confirmed' | 'unconfirmed',
  view?: FirestoreSettingsView,
): void {
  target.replaceChildren();
  if (access !== 'confirmed' || !view) {
    target.append(
      node(
        'p',
        access === 'checking'
          ? 'Checking recorded Firestore setup…'
          : 'Firestore setup could not be confirmed.',
      ),
    );
    if (access !== 'checking') {
      const hint = node('p', 'Check with ');
      hint.append(
        command('tmt remote status --layers'),
        document.createTextNode(' on the machine that runs Remote.'),
      );
      target.append(hint);
    }
    target.dataset.tone = access === 'checking' ? 'waiting' : 'blocked';
    return;
  }
  target.dataset.tone = 'review';
  if (!view.firestoreLayers.length) target.append(node('p', 'Firestore is not configured.'));
  for (const layer of view.firestoreLayers) {
    const group = node('section');
    group.className = 'firestore-layer';
    group.dataset.layer = layer.layer;
    const title = firestoreTitles[layer.layer];
    // Every-state interpretation: unsupported layers cannot work regardless of other evidence.
    if (layer.prerequisites.some((p) => p.item === 'support' && p.reason === 'not-implemented')) {
      const line = node('h3', `${title} — Not available in this release.`);
      group.append(line);
    } else {
      const heading = node('h3', `${title} `);
      heading.append(mark(layer.state));
      group.append(heading);
      const list = node('ul');
      for (const item of layer.prerequisites) {
        const row = node('li');
        row.dataset.item = item.item;
        row.append(node('span', `${firestoreItems[item.item]}: `), mark(item.state));
        if (item.reason) row.append(node('p', firestoreReasons[item.item]![item.reason]![0]));
        if (item.next) {
          const hint = node('p', 'Next command on the machine that runs Remote: ');
          hint.append(command(item.next));
          row.append(hint);
        }
        list.append(row);
      }
      group.append(list);
    }
    target.append(group);
  }
  const details = node('details');
  details.id = 'firestore-budget';
  details.append(node('summary', 'Firestore free-plan limits'));
  const published = view.firestoreBudget;
  details.append(node('h3', `Published limits as of ${published.readOn}`));
  const table = node('table');
  const head = node('tr');
  for (const text of ['Limit', 'Published allowance']) {
    const cell = node('th', text);
    cell.scope = 'col';
    head.append(cell);
  }
  const thead = node('thead');
  thead.append(head);
  table.append(thead);
  const body = node('tbody');
  for (const [key, label, per, bytes] of budget.publishedLimitRows) {
    const row = node('tr');
    const title = node('th', label);
    title.scope = 'row';
    row.append(title, node('td', allowance(published.limits[key]!, bytes, per)));
    body.append(row);
  }
  table.append(body);
  details.append(table);
  details.append(
    node(
      'p',
      'Google may change these limits. They are shared by the whole Firebase project. Daily limits reset around midnight Pacific time.',
    ),
  );
  details.append(
    node('p', "Remote can't see your actual usage; check it in the Firebase console."),
  );
  details.append(
    node(
      'p',
      `Each device warns at ${published.guard.warnPercent}% and stops at ${published.guard.refusePercent}% of its share of the daily limits.`,
    ),
  );
  target.append(details);
}
