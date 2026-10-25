/** Read-only settings presentation; all evidence was validated on the signed SDK lane. */
import { budget } from 'remote-browser-sdk';
import { browserUiClasses } from '@tmt/browser-ui/static';
import { firestoreTitles, firestoreItems, firestoreReasons, serviceCommands } from './firestore.js';
import { commandCopy } from './command-copy.js';
import type { FirestoreSettingsView } from './management.js';
const states = {
  enabled: ['✓', 'Enabled', 'working'],
  building: ['◌', 'Building', 'waiting'],
  'not-enabled': ['—', 'Not enabled', 'review'],
  unknown: ['○', 'Not checked', 'waiting'],
} as const;
function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = ''): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  element.textContent = text;
  return element;
}
function mark(state: keyof typeof states): HTMLElement {
  const [icon, text, tone] = states[state];
  const label = node('span');
  label.dataset.tone = tone;
  label.className = browserUiClasses.listState;
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
function link(text: string, href: string): HTMLAnchorElement {
  const anchor = node('a', text);
  anchor.href = href;
  anchor.target = '_blank';
  anchor.rel = 'noopener noreferrer';
  return anchor;
}
function checks(): HTMLElement {
  const section = node('section');
  section.className = 'firestore-layer';
  section.setAttribute('aria-label', 'Checks before an update');
  section.append(node('h3', 'Before an update'));
  const rows = [
    {
      id: 'version',
      title: 'Deployed version and required version',
      detail:
        'Version comparison is not available here. Check this Remote’s setup in this machine’s terminal before upgrading.',
      command: serviceCommands.status,
    },
    {
      id: 'changes',
      title: 'What will change',
      detail:
        'No deployment plan has been checked here. Preview the exact changes in this machine’s terminal.',
      command: serviceCommands.preview,
    },
    {
      id: 'permissions',
      title: 'Account permissions',
      detail:
        'Permissions have not been checked here. Use the Firebase account that manages the project in this machine’s terminal to check them.',
      command: serviceCommands.preview,
    },
  ];
  for (const row of rows) {
    const item = node('section');
    item.dataset.serviceCheck = row.id;
    const title = node('h4', `${row.title}: `);
    title.append(mark('unknown'));
    item.append(title, node('p', row.detail), commandCopy(row.command));
    section.append(item);
  }
  section.append(
    node(
      'p',
      'Replace <project-id> and <region> with your Firebase project and its region. The preview command does not authorize deployment.',
    ),
  );
  return section;
}
function guidance(): HTMLElement {
  const section = node('section');
  section.className = 'firestore-layer';
  section.append(node('h3', 'Where to check and make changes'));
  const steps = node('ol');
  const consoleStep = node(
    'li',
    'In the provider console, choose your Firebase project. Review its sign-in, database rules and usage. ',
  );
  consoleStep.append(link('Open Firebase console', 'https://console.firebase.google.com/'));
  const permissionStep = node(
    'li',
    'In the provider console, open Project settings → Users and permissions. Ask the project administrator for missing access. ',
  );
  permissionStep.append(
    link('Firebase roles and permissions', 'https://firebase.google.com/docs/projects/iam/roles'),
  );
  steps.append(
    node(
      'li',
      'In this machine’s terminal, install the Firebase CLI and sign in with the account that manages the project.',
    ),
    consoleStep,
    node(
      'li',
      'In this machine’s terminal, copy the preview command above to review changes and check permissions. Review and authorize any deployment there.',
    ),
    permissionStep,
    node(
      'li',
      'After you act in the terminal or provider console, return here and use Refresh values. That rechecks this machine’s saved setup facts; it does not run a live provider check.',
    ),
  );
  section.append(steps);
  return section;
}
function paidStorage(): HTMLElement {
  const paid = node(
    'p',
    'Paid Firebase Blaze usage and separate consent are required when file storage becomes available. ',
  );
  paid.append(
    link(
      'Firebase pricing plans',
      'https://firebase.google.com/docs/projects/billing/firebase-pricing-plans',
    ),
  );
  return paid;
}
function futureServices(includeStorage = false): HTMLElement {
  const section = node('section');
  section.className = 'firestore-layer';
  section.setAttribute('aria-label', 'Other services');
  section.append(node('h3', 'Other services'));
  if (includeStorage)
    section.append(
      node('p', 'File storage (paid add-on) — Not available in this release.'),
      paidStorage(),
    );
  section.append(node('p', 'Cloudflare — Coming soon.'));
  return section;
}
// This is presentation equality only: signed observations and their fences always run.
const painted = new WeakMap<HTMLElement, string>();
export function renderFirestore(
  target: HTMLElement,
  access: 'checking' | 'confirmed' | 'unconfirmed',
  view?: FirestoreSettingsView,
): void {
  const projection = JSON.stringify([access, view]);
  if (painted.get(target) === projection) return;
  painted.set(target, projection);
  const announcement = document.getElementById('firestore-announcement')!;
  const summary =
    access === 'checking'
      ? ''
      : access !== 'confirmed' || !view
        ? 'Service status could not be confirmed.'
        : !view.firestoreLayers.length
          ? 'Firestore is not configured.'
          : view.firestoreLayers
              .map(
                (layer) =>
                  `${firestoreTitles[layer.layer]}: ${layer.prerequisites.some((item) => item.item === 'support' && item.reason === 'not-implemented') ? 'Not available in this release' : states[layer.state][1]}.`,
              )
              .join(' ');
  // A refresh may show checking, but repeats of the same settled state stay quiet.
  if (summary && announcement.textContent !== summary) announcement.textContent = summary;
  target.replaceChildren();
  if (access !== 'confirmed' || !view) {
    target.append(
      node(
        'p',
        access === 'checking'
          ? 'Checking service status…'
          : 'Service status could not be confirmed.',
      ),
    );
    if (access !== 'checking') target.append(checks(), guidance(), futureServices(true));
    target.dataset.tone = access === 'checking' ? 'waiting' : 'blocked';
    return;
  }
  target.dataset.tone = 'review';
  target.append(
    node(
      'p',
      view.firestoreLayers.length
        ? 'Cloud service: Firebase (Firestore).'
        : 'Firestore is not configured.',
    ),
  );
  target.append(
    node(
      'p',
      'These setup facts come from this machine. They are not a live check of your Firebase project.',
    ),
  );
  for (const layer of view.firestoreLayers) {
    const group = node('section');
    group.className = 'firestore-layer';
    group.dataset.layer = layer.layer;
    const title = firestoreTitles[layer.layer];
    // Every-state interpretation: unsupported layers cannot work regardless of other evidence.
    if (layer.prerequisites.some((p) => p.item === 'support' && p.reason === 'not-implemented')) {
      const line = node('h3', `${title} — Not available in this release.`);
      group.append(line);
      if (layer.layer === 'attachments') group.append(paidStorage());
    } else {
      const heading = node('h3', `${title} `);
      heading.append(mark(layer.state));
      group.append(heading);
      const list = node('ul');
      for (const item of layer.prerequisites) {
        const row = node('li');
        row.dataset.item = item.item;
        const displayState =
          item.item === 'indexes' && item.state !== 'enabled'
            ? item.reason === 'building'
              ? 'building'
              : 'unknown'
            : item.state;
        row.append(node('span', `${firestoreItems[item.item]}: `), mark(displayState));
        if (item.reason) row.append(node('p', firestoreReasons[item.item]![item.reason]![0]));
        if (item.next) {
          const nextLine = firestoreReasons[item.item]![item.reason!]![2];
          const hint = node('p', nextLine ?? 'Run this in this machine’s terminal:');
          row.append(hint, commandCopy(item.next));
        }
        list.append(row);
      }
      group.append(list);
      if (layer.layer === 'sharing' && layer.state === 'enabled')
        group.append(
          node(
            'p',
            "Each client stops itself before the free-plan limits. TMT does not check the project's remaining quota, so Firebase may still refuse requests.",
          ),
        );
    }
    target.append(group);
  }
  target.append(
    checks(),
    guidance(),
    futureServices(!view.firestoreLayers.some((layer) => layer.layer === 'attachments')),
  );
  const details = node('details');
  details.id = 'firestore-budget';
  details.className = browserUiClasses.disclosure;
  details.append(node('summary', 'Firestore free-plan limits'));
  const published = view.firestoreBudget;
  details.append(node('h3', `Published limits as of ${published.readOn}`));
  const table = node('table');
  table.className = browserUiClasses.table;
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
