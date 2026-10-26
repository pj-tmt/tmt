/** A terminal/provider-console guide over the retained management observation, never a deploy runner. */
import { browserUiClasses as c } from '@tmt/browser-ui/static';
import { commandCopy } from './command-copy.js';
import { deploymentCommands, serviceCommands } from './service-commands.js';
import { renderFirestore } from './firestore-page.js';
import type { FirestoreSettingsView } from './management.js';
import type { RemoteRegistryPage } from './remote-registry-page.js';

export interface CreateGuideOptions {
  signal: AbortSignal;
  registry: RemoteRegistryPage;
  navigate(path: string): void;
  canRefresh(): boolean;
  refresh(owner: HTMLElement): Promise<void>;
}
function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = ''): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  element.textContent = text;
  return element;
}
function external(text: string, href: string): HTMLAnchorElement {
  const link = node('a', text);
  link.href = href;
  link.target = '_blank';
  link.rel = 'noopener noreferrer';
  return link;
}

export function createPage(
  target: HTMLElement,
  options: CreateGuideOptions,
): {
  observe(access: 'checking' | 'confirmed' | 'unconfirmed', view?: FirestoreSettingsView): void;
} {
  const { signal } = options;
  let type: 'local' | 'firebase' = 'local';
  let step = 1;
  const visited = new Set<number>();
  let rechecking = false;
  const buttons: HTMLButtonElement[] = [];
  const panels: HTMLElement[] = [];
  const titles = ['Type', 'Prepare', 'Details', 'Review', 'Check result'];
  const steps = node('ol');
  steps.className = 'create-steps';
  steps.setAttribute('aria-label', 'Create guide progress');
  const content = node('div');
  const actions = node('div');
  actions.className = `${c.actions} create-actions`;
  const action = (label: string, run: () => void, primary = false) => {
    const button = node('button');
    button.type = 'button';
    button.className = c.action;
    if (primary) button.dataset.variant = 'primary';
    const text = node('span', label);
    text.className = c.actionLabel;
    button.append(text);
    button.addEventListener('click', run, { signal });
    return button;
  };
  const field = (label: string, hint: string) => {
    const wrap = node('label');
    wrap.className = c.field;
    const name = node('span', label);
    name.className = c.fieldLabel;
    const input = node('input');
    input.className = c.fieldControl;
    input.autocomplete = 'off';
    input.spellcheck = false;
    const description = node('span', hint);
    description.className = c.fieldDescription;
    wrap.append(name, input, description);
    return { wrap, input };
  };
  for (const [index, title] of titles.entries()) {
    const item = node('li');
    const button = action(`${index + 1} ${title}`, () => show(index + 1));
    button.className = `${c.action} ${c.chip} create-step`;
    item.append(button);
    steps.append(item);
    buttons.push(button);
    const panel = node('section');
    panel.className = 'create-step-panel';
    panel.dataset.createStep = String(index + 1);
    panel.hidden = true;
    const heading = node('h2', title);
    heading.className = 'create-step-heading';
    panel.append(heading);
    panels.push(panel);
    content.append(panel);
  }
  const choose = node('fieldset');
  choose.className = c.choices;
  const legend = node('legend', 'Choose a connection type');
  legend.className = c.choicesLegend;
  choose.append(legend);
  for (const [value, label, detail] of [
    ['local', 'This machine', 'Use the door on this machine. No cloud account is needed.'],
    [
      'firebase',
      'Firebase · Free Spark plan',
      'Follow the terminal and console steps for an existing Firebase project.',
    ],
    ['cloudflare', 'Cloudflare · Coming soon', 'Not available in this release.'],
  ]) {
    const row = node('label');
    row.className = c.choice;
    const radio = node('input');
    radio.type = 'radio';
    radio.name = 'create-type';
    radio.value = value!;
    radio.className = c.radio;
    radio.checked = value === type;
    radio.disabled = value === 'cloudflare';
    radio.addEventListener(
      'change',
      () => {
        if (radio.checked && (value === 'local' || value === 'firebase')) {
          type = value;
          visited.clear();
          show(1, false);
        }
      },
      { signal },
    );
    const copy = node('span');
    const name = node('span', label);
    name.className = c.choiceText;
    const description = node('span', detail);
    description.className = c.choiceDescription;
    copy.append(name, description);
    row.append(radio, copy);
    choose.append(row);
  }
  panels[0]!.append(choose);

  panels[1]!.querySelector('h2')!.textContent = 'Prepare Firebase';
  const card = node('div');
  card.className = `${c.card} create-prerequisites`;
  card.append(
    node(
      'p',
      'Not checked: the browser cannot check these. Do them in this machine’s terminal or the Firebase console.',
    ),
  );
  const checks = node('ul');
  checks.className = 'create-checks';
  for (const [label, detail, commands] of [
    [
      'Node.js',
      'Check the installed version in this machine’s terminal.',
      [serviceCommands.nodeVersion],
    ],
    [
      'Firebase CLI',
      'Check or install the supported CLI in this machine’s terminal.',
      [serviceCommands.firebaseVersion, serviceCommands.firebaseInstall],
    ],
    [
      'Your Firebase account',
      'Sign in from this machine’s terminal. Firebase opens its own sign-in page.',
      [serviceCommands.firebaseLogin],
    ],
    [
      'Project and Authentication',
      'In the Firebase console, choose an existing Spark project and select Authentication → Get started.',
      [],
    ],
  ] as const) {
    const row = node('li');
    const mark = node('span', '◌');
    mark.className = 'create-check-mark';
    mark.setAttribute('role', 'img');
    mark.setAttribute('aria-label', 'Not checked');
    const copy = node('div');
    copy.append(node('strong', label), node('p', detail));
    for (const command of commands) copy.append(commandCopy(command));
    if (!commands.length)
      copy.append(external('Open Firebase console', 'https://console.firebase.google.com/'));
    row.append(mark, copy);
    checks.append(row);
  }
  card.append(checks);
  panels[1]!.append(card);

  const firebaseInputs = node('div');
  const project = field(
    'Firebase project ID',
    'Use your existing project ID from the Firebase console. No account inventory is read here.',
  );
  const region = field(
    'Database region',
    'The region cannot change after authorization. Use the region of an existing database.',
  );
  const invalid = node('p');
  invalid.className = c.fieldError;
  invalid.setAttribute('role', 'alert');
  firebaseInputs.append(
    project.wrap,
    region.wrap,
    invalid,
    node(
      'p',
      'Defaults use the (default) database, a new Web App and dedicated Hosting site, and anonymous sign-in. No paid features are set up.',
    ),
  );
  const name = node('p');
  const localState = node('p');
  const rename = action('Name in this browser', () => options.registry.renameCurrent(rename));
  const unsubscribe = options.registry.watch((view) => {
    name.textContent = `Name in this browser: ${view.current.name}. This names the current known Remote only.`;
    localState.textContent = `This door’s observed connection: ${view.current.connection.text}.`;
  });
  signal.addEventListener('abort', unsubscribe, { once: true });
  panels[2]!.append(
    firebaseInputs,
    name,
    rename,
    node('p', 'Names stay in this browser. A browser name is never a Firebase deployment input.'),
  );
  const review = node('div');
  panels[3]!.append(review);
  const result = panels[4]!;
  const completion = node('p');
  const saved = node('div');
  saved.className = 'create-saved-facts';
  const announcement = node('p');
  announcement.className = c.visuallyHidden;
  announcement.setAttribute('role', 'status');
  const refreshFeedback = node('p');
  refreshFeedback.setAttribute('role', 'status');
  const refresh = action('Recheck saved facts', () => {
    if (signal.aborted || rechecking || !options.canRefresh()) return;
    rechecking = true;
    refresh.disabled = true;
    refreshFeedback.textContent = '';
    void options
      .refresh(refresh)
      .catch(() => {
        if (!signal.aborted)
          refreshFeedback.textContent =
            'Saved facts could not be checked. Keep the original terminal result.';
      })
      .finally(() => {
        if (signal.aborted) return;
        rechecking = false;
        refresh.disabled = !options.canRefresh();
      });
  });
  refresh.disabled = true;
  const overview = action('Open this Remote', () => options.navigate('/remote/overview'));
  const invites = action('Invite someone', () => options.navigate('/remote/invites'));
  const resultLinks = node('div');
  resultLinks.className = `${c.actions} create-actions`;
  resultLinks.append(overview, invites);
  result.append(completion, localState, announcement, saved, refresh, refreshFeedback, resultLinks);
  const back = action('Back', () => show(step === 3 && type === 'local' ? 1 : step - 1));
  const next = action('Continue', () => show(step === 1 && type === 'local' ? 3 : step + 1), true);
  const cancel = action('Cancel', () => options.navigate('/remote/'));
  actions.append(cancel, back, next);
  target.append(steps, content, actions);

  function show(selected: number, focus = true): void {
    if (signal.aborted || selected < 1 || selected > 5 || (selected === 2 && type === 'local'))
      return;
    let commands: ReturnType<typeof deploymentCommands> | undefined;
    if (type === 'firebase' && selected >= 4) {
      try {
        commands = deploymentCommands(project.input.value, region.input.value);
      } catch (error) {
        show(3, false);
        invalid.textContent =
          error instanceof Error ? error.message : 'Check the project and region.';
        project.input.focus();
        return;
      }
    }
    invalid.textContent = '';
    if (selected !== step) visited.add(step);
    step = selected;
    panels.forEach((panel, index) => {
      panel.hidden = index + 1 !== step;
    });
    buttons.forEach((button, index) => {
      const number = index + 1;
      const state = number === step ? 'current' : visited.has(number) ? 'visited' : 'upcoming';
      button.dataset.stepState = state;
      button.setAttribute('aria-label', `${number} ${titles[index]} · ${state}`);
      if (number === step) button.setAttribute('aria-current', 'step');
      else button.removeAttribute('aria-current');
      button.disabled = type === 'local' && number === 2;
    });
    firebaseInputs.hidden = type !== 'firebase';
    review.replaceChildren();
    if (commands) {
      review.append(
        node('h3', 'Preview changes and permissions · Not checked'),
        node(
          'p',
          'Copy this preview into this machine’s terminal with the Firebase account that manages the project. It does not authorize deployment.',
        ),
        commandCopy(commands.preview),
      );
      const terminal = node('section');
      terminal.append(
        node('h3', 'Your terminal step'),
        node(
          'p',
          'In your own terminal, review the captured changes, including replacement of Security rules or Hosting files, and answer y/N there. The browser does not approve or execute this step.',
        ),
        commandCopy(commands.deploy),
      );
      review.append(terminal);
    } else
      review.append(
        node(
          'p',
          'This machine uses its current door. No cloud account or deployment is needed. Open Overview to check the connection.',
        ),
      );
    completion.textContent =
      type === 'firebase'
        ? 'Deployment completion · Not checked. A copied command or enabled layer does not verify a completed deployment. For a partial or unknown result, keep the printed record and follow the original run’s terminal guidance; do not reset it or repeat unknown work.'
        : 'No cloud deployment is involved. The connection below is the existing observation, not a new setup check.';
    saved.hidden = type !== 'firebase';
    refresh.hidden = type !== 'firebase';
    invites.hidden = type !== 'firebase';
    back.hidden = step === 1;
    cancel.hidden = step !== 1;
    next.hidden = step === 5;
    if (focus) {
      const heading = panels[step - 1]!.querySelector('h2')!;
      heading.tabIndex = -1;
      heading.focus();
    }
  }
  show(1, false);
  return {
    observe(access, view) {
      if (signal.aborted) return;
      refresh.disabled = rechecking || access === 'checking' || !options.canRefresh();
      renderFirestore(saved, access, view, { commands: false, announcement, prefix: 'create' });
    },
  };
}
