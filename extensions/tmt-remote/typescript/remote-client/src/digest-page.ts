/**
 * The Digest web view: the usage report as sections of charts and tables, ordered by the questions
 * the PM reads it for (runway, who burns, idle or gate-bound), then who talks to whom, one block
 * naming the questions the usage read cannot answer yet (throughput, gates, cost per merged PR,
 * machine) and the method. It depends only on
 * `DigestSource`; the product source says "not in the browser yet" until Core serves the read
 * (#4325), and a sample source is only ever passed in by tests, the gallery and dev.
 */
import {
  compactCount,
  createMatrix,
  createSeriesChart,
  createStrips,
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
  allowance,
  exchange,
  historyCharts,
  rankRows,
  resetsIn,
  tokenSeries,
} from './digest-view.js';
import { notAvailable } from './not-available.js';

export { compactCount as formatTokens };

/** Distinct series a chart draws; the rest fold into one neutral `Other`. */
const SERIES_KEPT = 5;

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

function section(title: string, id: string, ...body: Node[]): HTMLElement {
  const node = el('section', 'digest-section');
  node.id = id;
  node.setAttribute('aria-labelledby', `${id}-title`);
  const heading = el('h2', 'home-heading', title);
  heading.id = `${id}-title`;
  node.append(heading, ...body);
  return node;
}

/** Two blocks side by side where the shell is wide, stacked where it is not. */
function split(className: string, ...blocks: Node[]): HTMLElement {
  const node = el('div', `digest-split ${className}`);
  node.append(...blocks);
  return node;
}

function table(head: readonly string[], rows: readonly (readonly string[])[]): HTMLTableElement {
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
    const name = el('th', undefined, first);
    name.scope = 'row';
    line.append(name, ...rest.map((text) => el('td', undefined, text)));
    body.append(line);
  }
  node.append(body);
  return node;
}

function allowanceSection(doc: UsageDocument): HTMLElement {
  const rows = allowance(doc);
  const charts = historyCharts(doc);
  if (!rows.length) {
    return section(
      'Runway',
      'digest-allowance',
      el('p', 'usage-note', 'No weekly window has been observed yet.'),
    );
  }
  const summary = table(
    ['Provider', 'Left', 'Elapsed', 'Pace', 'Resets in'],
    rows.map((row) => [
      row.provider,
      `${row.leftPercent}%`,
      row.elapsedPercent === null ? '—' : `${Math.round(row.elapsedPercent)}%`,
      row.paceDeltaPoints === null
        ? '—'
        : `${row.paceDeltaPoints > 0 ? '+' : ''}${row.paceDeltaPoints.toFixed(1)} pts`,
      row.stale ? 'stale' : resetsIn(row.resetsAtMs, doc.observedAtMs),
    ]),
  );
  if (!charts.length) {
    return section(
      'Runway',
      'digest-allowance',
      summary,
      el('p', 'usage-note', 'The weekly history is not served yet; this shows the latest window.'),
    );
  }
  const history = el('div', 'digest-charts');
  history.append(...charts.map((chart) => createSeriesChart(chart)));
  return section('Runway', 'digest-allowance', split('digest-split-runway', summary, history));
}

function tokensBlock(doc: UsageDocument): HTMLElement {
  const data = tokenSeries(doc);
  const block = el('div', 'digest-block');
  block.id = 'digest-tokens';
  const host = el('div');
  const toggle = el('button', 'tmt-ui-toggle');
  toggle.type = 'button';
  const mark = el('span', 'tmt-ui-toggle-indicator');
  mark.setAttribute('aria-hidden', 'true');
  toggle.append(mark, el('span', undefined, 'Share of total (100%)'));
  let share = false;
  const draw = () => {
    toggle.setAttribute('aria-pressed', String(share));
    mark.textContent = share ? '✓' : '';
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
  toggle.addEventListener('click', () => {
    share = !share;
    draw();
  });
  draw();
  block.append(toggle, host);
  if (data.partial)
    block.append(
      el('p', 'usage-note', 'Some buckets are incomplete: they are lower bounds, not errors.'),
    );
  return block;
}

function rankingBlock(doc: UsageDocument): HTMLElement {
  const rows = rankRows(doc);
  const block = el('div', 'digest-block');
  block.id = 'digest-ranking';
  block.append(
    el('h3', 'tmt-ui-chart-title', `Last ${doc.since} by seat`),
    rows.length
      ? table(
          ['Seat', 'Tokens', 'Share'],
          rows.map((row) => [
            row.partial ? `${row.name} (partial)` : row.name,
            compactCount(row.tokens),
            `${row.share}%`,
          ]),
        )
      : el('p', 'usage-note', 'No seat has token observations in this range.'),
  );
  return block;
}

function burnSection(doc: UsageDocument): HTMLElement {
  return section(
    'Who burns',
    'digest-burn',
    split('digest-split-burn', tokensBlock(doc), rankingBlock(doc)),
  );
}

function exchangeSection(doc: UsageDocument): HTMLElement {
  const flow = exchange(doc);
  return section(
    'Interaction volume: who talks to whom',
    'digest-exchange',
    flow
      ? createMatrix({
          title: `Messages, last ${doc.since}`,
          participants: flow.participants,
          value: flow.value,
          sum: true,
        })
      : el('p', 'usage-note', 'Message traffic is not available in this read.'),
  );
}

function activitySection(doc: UsageDocument): HTMLElement | undefined {
  const view = activity(doc);
  if (!view) return undefined;
  const label = (at: number) =>
    new Date(view.starts[at]!).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
  return section(
    'Idle, stalled or gate-bound',
    'digest-activity',
    createStrips({
      title: 'Tokens per hour, oldest on the left',
      rows: view.rows,
      slotCount: 24,
      slotLabel: label,
      ends: ['24 h ago', 'now'],
    }),
    el(
      'p',
      'usage-note',
      'Token flow only. Board row state and age, pane evidence and what each seat waits on are not in the usage read yet.',
    ),
  );
}

/** The questions the usage read cannot answer yet: one line each, never a zero or an empty chart. */
function missingSection(): HTMLElement {
  const lines: [string, string][] = [
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
  for (const [term, source] of lines)
    list.append(el('dt', undefined, term), el('dd', undefined, source));
  return section('Not in the usage read yet', 'digest-missing', list);
}

function methodSection(): HTMLElement {
  const lines = [
    'Tokens are input plus output as Core reports them; cached input is part of input and is not counted again.',
    'An incomplete bucket is a lower bound. A seat with no evidence for part of the range is marked partial, never shown as zero.',
    'Allowance windows are the account percent as the provider reports it, in whole percent; usage on other machines is not seen.',
    'Message counts include talk and reply only.',
  ];
  return section(
    'Method and caveats',
    'digest-method',
    ...lines.map((text) => el('p', 'usage-note', text)),
  );
}

export function digestPage(source: DigestSource = digestSource): void {
  const tabs = document.getElementById('usage-ranges');
  const panel = document.getElementById('digest-sections');
  const note = document.getElementById('usage-note');
  if (!tabs || !panel || !note) return;
  let current: UsageRange = USAGE_RANGES[0];
  let generation = 0;
  const buttons = USAGE_RANGES.map((range) => {
    const button = el('button', 'tmt-ui-tab');
    button.type = 'button';
    button.setAttribute('role', 'tab');
    button.setAttribute('aria-controls', 'usage-panel');
    button.dataset.range = range;
    button.append(el('span', 'tmt-ui-tab-label', range));
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
    if (report.unavailable || !report.document) {
      panel!.replaceChildren();
      return notAvailable(note!, report.unavailable ?? 'Usage is not available.');
    }
    const doc = report.document;
    panel!.replaceChildren(
      ...[
        allowanceSection(doc),
        burnSection(doc),
        activitySection(doc),
        exchangeSection(doc),
        missingSection(),
        methodSection(),
      ].filter((node): node is HTMLElement => node !== undefined),
    );
    if (report.sample) {
      notAvailable(
        note!,
        'Sample data: Digest has no web read of your usage yet. `tmt digest usage report` shows the real numbers.',
      );
    } else {
      note!.replaceChildren();
      note!.hidden = true;
    }
  }
  void show(current);
}
