/**
 * The Digest web view: the usage report as cards of charts and tables. The arrangement is the
 * team's usage at a glance: provider allowance (meter, weekly history, pace), who burns (tokens
 * over time beside the ranking), activity by seat, then who talks to whom beside the data the
 * read does not carry yet and the method. It depends only on `DigestSource`; the product source
 * says "not in the browser yet" until Core serves the read (#4325), and a sample source is only
 * ever passed in by tests, the gallery and dev.
 */
import {
  compactCount,
  createMatrix,
  createSeriesChart,
  createStrips,
  stripsLegend,
  topLayers,
} from '@tmt/browser-ui/chart';
import { pageLoading } from './page-loading.js';
import {
  digestSource,
  USAGE_RANGES,
  type DigestSource,
  type UsageDocument,
  type UsageRange,
} from './digest-source.js';
import {
  activity,
  exchange,
  providers,
  rankRows,
  resetsIn,
  tokenSeries,
  type ProviderView,
} from './digest-view.js';
import { notAvailable } from './not-available.js';

export { compactCount as formatTokens };

/** Distinct series a chart draws; the rest fold into one neutral `Other`. */
const SERIES_KEPT = 5;
/** The neutral palette step of the folded `Other` layer (`tmt-ui-chart-s7`). */
const OTHER_STEP = 7;

function el<K extends keyof HTMLElementTagNameMap>(
  name: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(name);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** A flat sheet with a sentence-case title and an optional right-hand slot in its head. */
function card(
  id: string,
  title: string,
  body: readonly Node[],
  options: { actions?: Node; className?: string } = {},
): HTMLElement {
  const node = el(
    'section',
    options.className ? `digest-card ${options.className}` : 'digest-card',
  );
  node.id = id;
  node.setAttribute('aria-labelledby', `${id}-title`);
  const heading = el('h2', 'digest-card-title', title);
  heading.id = `${id}-title`;
  const head = el('div', 'digest-card-head');
  head.append(heading);
  if (options.actions) head.append(options.actions);
  node.append(head, ...body);
  return node;
}

function row(className: string, ...cards: Node[]): HTMLElement {
  const node = el('div', `digest-row ${className}`);
  node.append(...cards);
  return node;
}

function table(head: readonly string[], rows: readonly (readonly (string | Node)[])[]) {
  const node = el('table', 'usage-table');
  const header = el('tr');
  for (const text of head) {
    const cell = el('th', undefined, text);
    cell.scope = 'col';
    header.append(cell);
  }
  const thead = el('thead');
  thead.append(header);
  node.append(thead);
  const body = el('tbody');
  for (const [first, ...rest] of rows) {
    const line = el('tr');
    const name = el('th');
    name.scope = 'row';
    name.append(first!);
    line.append(
      name,
      ...rest.map((value) => {
        const cell = el('td');
        cell.append(value);
        return cell;
      }),
    );
    body.append(line);
  }
  node.append(body);
  return node;
}

/** A segmented control of buttons; `aria-pressed` marks the active one. */
function segments<T extends string>(
  label: string,
  options: readonly { value: T; text: string }[],
  initial: T,
  change: (value: T) => void,
): HTMLElement {
  const group = el('div', 'digest-segments');
  group.setAttribute('role', 'group');
  group.setAttribute('aria-label', label);
  const buttons = options.map((option) => {
    const button = el('button', 'digest-segment', option.text);
    button.type = 'button';
    button.addEventListener('click', () => {
      sync(option.value);
      change(option.value);
    });
    return { button, value: option.value };
  });
  const sync = (active: T) => {
    for (const { button, value } of buttons)
      button.setAttribute('aria-pressed', String(value === active));
  };
  sync(initial);
  group.append(...buttons.map(({ button }) => button));
  return group;
}

function providerCard(view: ProviderView, nowMs: number): HTMLElement {
  const resets = view.stale ? 'stale' : resetsIn(view.resetsAtMs, nowMs);
  const meter = el('div', 'digest-meter');
  meter.setAttribute('role', 'img');
  meter.setAttribute('aria-label', `${view.leftPercent}% of the weekly allowance left`);
  const fill = el('div', `digest-meter-fill tmt-ui-chart-s${view.style}`);
  fill.style.setProperty('--digest-fill', String(Math.max(0, Math.min(100, view.leftPercent))));
  meter.append(fill);
  const figure = el('div', 'digest-provider-figure');
  figure.append(
    el('p', 'digest-big', `${view.leftPercent}%`),
    el('p', 'digest-big-label', 'allowance left'),
    meter,
  );
  const history = view.chart
    ? createSeriesChart(view.chart)
    : el('p', 'usage-note', 'The weekly history is not served yet; this shows the latest window.');
  const stat = (label: string, value: string) => {
    const cell = el('div', 'digest-stat');
    cell.append(el('dt', undefined, label), el('dd', undefined, value));
    return cell;
  };
  const stats = el('dl', 'digest-stats');
  stats.append(
    stat('Elapsed', view.elapsedPercent === null ? '—' : `${Math.round(view.elapsedPercent)}%`),
    stat(
      'Pace',
      view.paceDeltaPoints === null
        ? '—'
        : `${view.paceDeltaPoints > 0 ? '+' : ''}${view.paceDeltaPoints.toFixed(1)} pts`,
    ),
    stat('Resets in', resets),
  );
  const body = el('div', 'digest-provider-body');
  body.append(figure, history);
  return card(`digest-provider-${view.line || 'account'}`, view.provider, [body, stats], {
    actions: el('span', 'digest-card-meta', `Resets in ${resets}`),
    className: 'digest-provider',
  });
}

function allowanceRow(doc: UsageDocument): HTMLElement {
  const views = providers(doc);
  const node = views.length
    ? row('digest-row-providers', ...views.map((view) => providerCard(view, doc.observedAtMs)))
    : row(
        'digest-row-providers',
        card('digest-provider-none', 'Allowance', [
          el('p', 'usage-note', 'No weekly window has been observed yet.'),
        ]),
      );
  node.id = 'digest-allowance';
  return node;
}

function tokensCard(doc: UsageDocument): HTMLElement {
  const data = tokenSeries(doc);
  const body = el('div', 'digest-card-body');
  const host = el('div');
  let share = false;
  const draw = () => {
    host.replaceChildren(
      createSeriesChart({
        title: share ? 'Share of tokens by seat' : 'Tokens by seat',
        kind: share ? 'share' : 'stack',
        x: data.x,
        series: topLayers(data.layers, SERIES_KEPT),
        empty: 'No token observations in this range.',
      }),
    );
  };
  draw();
  body.append(host);
  if (data.partial)
    body.append(
      el('p', 'usage-note', 'Some buckets are incomplete: they are lower bounds, not errors.'),
    );
  const scale = segments(
    'Scale',
    [
      { value: 'absolute', text: 'Absolute' },
      { value: 'share', text: 'Share' },
    ],
    'absolute',
    (value) => {
      share = value === 'share';
      draw();
    },
  );
  return card('digest-tokens', 'Token usage', [body], { actions: scale });
}

function rankingCard(doc: UsageDocument): HTMLElement {
  const rows = rankRows(doc);
  // The seats the token chart draws as hues keep that hue here; the rest share the neutral step.
  const hue = new Map(
    topLayers(tokenSeries(doc).layers, SERIES_KEPT)
      .filter((layer) => layer.key !== 'other')
      .map((layer, at) => [layer.key, at]),
  );
  const largest = Math.max(1, ...rows.map((entry) => entry.tokens));
  const bar = (id: string, tokens: number) => {
    const node = el('span', `digest-bar tmt-ui-chart-s${hue.get(id) ?? OTHER_STEP}`);
    node.style.setProperty('--digest-bar', String(Math.round((tokens / largest) * 100)));
    node.setAttribute('aria-hidden', 'true');
    return node;
  };
  return card(
    'digest-ranking',
    `Last ${doc.since} by seat`,
    rows.length
      ? [
          table(
            ['Seat', '', 'Tokens', 'Share'],
            rows.map((entry) => [
              entry.partial ? `${entry.name} (partial)` : entry.name,
              bar(entry.id, entry.tokens),
              compactCount(entry.tokens),
              `${entry.share}%`,
            ]),
          ),
        ]
      : [el('p', 'usage-note', 'No seat has token observations in this range.')],
  );
}

function activityCard(doc: UsageDocument): HTMLElement | undefined {
  const view = activity(doc);
  if (!view) return undefined;
  const clock = (at: number) =>
    new Date(at).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
  const day = (at: number) =>
    new Date(at).toLocaleDateString('en-GB', { weekday: 'short', day: 'numeric', month: 'short' });
  const hourly = view.slot === 'hour';
  const range = (at: number) => {
    const start = view.starts[at]!;
    return hourly ? `${clock(start)}–${clock(start + 3_600_000)}` : day(start);
  };
  return card(
    'digest-activity',
    'Activity by seat',
    [
      createStrips({
        title: hourly
          ? 'Tokens per hour, oldest on the left'
          : 'Tokens per day, oldest on the left',
        rows: view.rows,
        slotCount: view.starts.length,
        slotRange: range,
        ends: hourly ? ['24 h ago', 'now'] : [`${view.starts.length} days ago`, 'today'],
        legend: false,
      }),
      el(
        'p',
        'usage-note',
        'Token flow only. Board row state and age, pane evidence and what each seat waits on are not in the usage read yet.',
      ),
    ],
    { actions: stripsLegend() },
  );
}

function exchangeCard(doc: UsageDocument): HTMLElement {
  const flow = exchange(doc);
  return card('digest-exchange', 'Messages between seats', [
    flow
      ? createMatrix({
          title: `Messages, last ${doc.since}`,
          participants: flow.participants,
          value: flow.value,
          sum: true,
        })
      : el('p', 'usage-note', 'Message traffic is not available in this read.'),
  ]);
}

/** The sources the usage read does not carry yet, one line each, with the method below. */
function coverageCard(): HTMLElement {
  const gaps: [string, string][] = [
    [
      'Throughput',
      'merged pull requests per provider and publishes in flight (GitHub, from the PR Agent lines)',
    ],
    [
      'Gates and queue',
      'green-not-armed and review waits, merge-queue entries and merge-group failures (GitHub)',
    ],
    [
      'Cost per merged PR',
      'merged pull requests joined to the allowance points burned in the same window',
    ],
    ['Machine', 'load, free disk and the biggest worktrees with their owners'],
  ];
  const list = el('dl', 'digest-gaps');
  for (const [term, source] of gaps) {
    const line = el('div', 'digest-gap');
    line.append(
      el('dt', undefined, term),
      el('dd', 'digest-badge', 'Not connected'),
      el('dd', 'digest-gap-source', source),
    );
    list.append(line);
  }
  const method = el('details', 'digest-method');
  method.id = 'digest-method';
  method.append(
    el('summary', undefined, 'Method & caveats'),
    ...[
      'Tokens are input plus output as Core reports them; cached input is part of input and is not counted again.',
      'An incomplete bucket is a lower bound. A seat with no evidence for part of the range is marked partial, never shown as zero.',
      'Allowance windows are the account percent as the provider reports it, in whole percent; usage on other machines is not seen.',
      'Message counts include talk and reply only.',
    ].map((text) => el('p', 'usage-note', text)),
  );
  return card('digest-missing', 'Data coverage', [list, method]);
}

export function digestPage(source: DigestSource = digestSource): void {
  const tabs = document.getElementById('usage-ranges');
  const panel = document.getElementById('digest-sections');
  const note = document.getElementById('usage-note');
  const badge = document.getElementById('usage-badge');
  if (!tabs || !panel || !note || !badge) return;
  let current: UsageRange = USAGE_RANGES[0];
  let generation = 0;
  const buttons = USAGE_RANGES.map((range) => {
    const button = el('button', 'digest-segment', range);
    button.type = 'button';
    button.setAttribute('role', 'tab');
    button.setAttribute('aria-controls', 'usage-panel');
    button.dataset.range = range;
    button.addEventListener('click', () => void show(range));
    button.addEventListener('keydown', (event) => {
      const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
      if (!step) return;
      event.preventDefault();
      const next = USAGE_RANGES.indexOf(current) + step;
      const range = USAGE_RANGES[(next + USAGE_RANGES.length) % USAGE_RANGES.length]!;
      void show(range).then(() => buttons[USAGE_RANGES.indexOf(range)]!.focus());
    });
    return button;
  });
  tabs.replaceChildren(...buttons);
  async function show(range: UsageRange) {
    current = range;
    const mine = ++generation;
    for (const button of buttons) {
      const selected = button.dataset.range === range;
      button.setAttribute('aria-selected', String(selected));
      button.tabIndex = selected ? 0 : -1;
    }
    panel!.replaceChildren(pageLoading());
    const report = await source.report(range);
    if (mine !== generation) return;
    badge!.hidden = true;
    if (report.unavailable || !report.document) {
      panel!.replaceChildren();
      return notAvailable(note!, report.unavailable ?? 'Usage is not available.');
    }
    const doc = report.document;
    const activityBlock = activityCard(doc);
    panel!.replaceChildren(
      allowanceRow(doc),
      row('digest-row-burn', tokensCard(doc), rankingCard(doc)),
      ...(activityBlock ? [activityBlock] : []),
      row('digest-row-coverage', exchangeCard(doc), coverageCard()),
    );
    if (report.sample) {
      badge!.hidden = false;
      notAvailable(note!, 'Usage preview. Real numbers: `tmt digest usage report`.');
    } else {
      note!.replaceChildren();
      note!.hidden = true;
    }
  }
  void show(current);
}
