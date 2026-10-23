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
  type DigestReport,
  type DigestSource,
  type UsageDocument,
  type UsageRange,
} from './digest-source.js';
import {
  activity,
  exchange,
  providers,
  ranking,
  rankRows,
  resetsIn,
  tokenSeries,
  type ProviderView,
} from './digest-view.js';
import { notAvailable } from './not-available.js';
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { whileVisible } from './owner-reads.js';

/** The terminal Digest's own cadence: the usage read refreshes once a minute. */
const REFRESH_MS = 60_000;

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
    options.className ? `${ui.card} digest-card ${options.className}` : `${ui.card} digest-card`,
  );
  node.id = id;
  node.setAttribute('aria-labelledby', `${id}-title`);
  const heading = el('h2', ui.cardTitle, title);
  heading.id = `${id}-title`;
  const head = el('div', ui.cardHead);
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
  const node = el('table', ui.table);
  node.dataset.numeric = '';
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

/** The leaf's boxed tabs as a two-way choice for one panel; `aria-selected` marks the active one. */
function tabChoice<T extends string>(
  label: string,
  controls: string,
  options: readonly { value: T; text: string }[],
  initial: T,
  change: (value: T) => void,
): HTMLElement {
  const header = el('div', ui.tabsHeader);
  const list = el('div', ui.tabList);
  list.setAttribute('role', 'tablist');
  list.setAttribute('aria-label', label);
  const buttons = options.map((option) => {
    const button = el('button', ui.tab, option.text);
    button.type = 'button';
    button.setAttribute('role', 'tab');
    button.setAttribute('aria-controls', controls);
    button.addEventListener('click', () => {
      sync(option.value);
      change(option.value);
    });
    return { button, value: option.value };
  });
  const sync = (active: T) => {
    for (const { button, value } of buttons)
      button.setAttribute('aria-selected', String(value === active));
  };
  sync(initial);
  list.append(...buttons.map(({ button }) => button));
  header.append(list);
  return header;
}

/** One numeric fact as the leaf's status tile: a label over a large value. Inside a card the tiles
 * are bare (`data-bare` on the group); `series` adds a dot in that chart series' colour, `hero` sets
 * the value at the page's first-read size. A tile without a tone draws no dot. */
function tile(
  label: string,
  value: string,
  options: { hero?: boolean; series?: number } = {},
): HTMLElement {
  const node = el('div', ui.statusTile);
  const term = el('dt', ui.statusTileLabel, label);
  const figure = el('dd', ui.statusTileValue);
  figure.dataset.numeric = value === '0' ? 'zero' : 'count';
  if (options.hero) figure.dataset.size = 'hero';
  if (options.series !== undefined) {
    const dot = el('span', `${ui.statusTileDot} tmt-ui-chart-s${options.series}`);
    dot.setAttribute('aria-hidden', 'true');
    figure.append(dot);
  }
  figure.append(value);
  node.append(term, figure);
  return node;
}

/** A passive label for a state that is not an action: the leaf's chip without a remove control. */
function passiveChip(text: string): HTMLElement {
  const node = el('span', ui.chip);
  node.append(el('span', ui.chipLabel, text));
  return node;
}

/** A group of bare tiles for use inside a card. */
function bareTiles(...tiles: HTMLElement[]): HTMLElement {
  const group = el('dl', ui.statusTiles);
  group.dataset.bare = '';
  group.append(...tiles);
  return group;
}

function providerCard(view: ProviderView, nowMs: number): HTMLElement {
  const resets = view.stale ? 'stale' : resetsIn(view.resetsAtMs, nowMs);
  const meter = el('div', ui.meter);
  meter.setAttribute('role', 'img');
  meter.setAttribute('aria-label', `${view.leftPercent}% of the weekly allowance left`);
  const fill = el('div', `${ui.meterFill} tmt-ui-chart-s${view.style}`);
  fill.style.setProperty(
    '--tmt-ui-meter-fill',
    String(Math.max(0, Math.min(100, view.leftPercent))),
  );
  meter.append(fill);
  const figure = el('div', 'digest-provider-figure');
  figure.append(
    bareTiles(tile('Allowance left', `${view.leftPercent}%`, { hero: true, series: view.style })),
    meter,
  );
  const history = view.chart
    ? createSeriesChart(view.chart)
    : el('p', 'usage-note', 'The weekly history is not served yet; this shows the latest window.');
  const stats = bareTiles(
    tile('Elapsed', view.elapsedPercent === null ? '—' : `${Math.round(view.elapsedPercent)}%`),
    tile(
      'Pace',
      view.paceDeltaPoints === null
        ? '—'
        : `${view.paceDeltaPoints > 0 ? '+' : ''}${view.paceDeltaPoints.toFixed(1)} pts`,
    ),
    tile('Resets in', resets),
  );
  const body = el('div', 'digest-provider-body');
  body.append(figure, history);
  return card(`digest-provider-${view.line || 'account'}`, view.provider, [body, stats], {
    actions: el('span', ui.cardMeta, `Resets in ${resets}`),
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

function tokensCard(doc: UsageDocument, scale: { share: boolean }): HTMLElement {
  const data = tokenSeries(doc);
  const body = el('div', 'digest-card-body');
  const host = el('div');
  const draw = () => {
    host.replaceChildren(
      createSeriesChart({
        title: scale.share ? 'Share of tokens by seat' : 'Tokens by seat',
        kind: scale.share ? 'share' : 'stack',
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
  host.id = 'digest-tokens-chart';
  const control = tabChoice(
    'Scale',
    host.id,
    [
      { value: 'absolute', text: 'Absolute' },
      { value: 'share', text: 'Share' },
    ],
    scale.share ? 'share' : 'absolute',
    (value) => {
      scale.share = value === 'share';
      draw();
    },
  );
  return card('digest-tokens', 'Token usage', [body], { actions: control });
}

/** A seat's name on one line, the full name on hover, `*` after a partial one. */
function seatLabel(name: string, partial: boolean): HTMLElement {
  const node = el('span', 'digest-seat', name);
  node.title = partial ? `${name} (partial)` : name;
  if (partial) node.append(el('span', 'digest-partial', '*'));
  return node;
}

function rankingCard(doc: UsageDocument): HTMLElement {
  const rows = rankRows(doc);
  const view = ranking(rows);
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
  if (!rows.length)
    return card('digest-ranking', `Last ${doc.since} by seat`, [
      el('p', 'usage-note', 'No seat has token observations in this range.'),
    ]);
  const host = el('div', 'digest-ranking-table');
  host.id = 'digest-ranking-rows';
  const draw = (all: boolean) => {
    const listed = all ? rows : view.top;
    const lines: (string | Node)[][] = listed.map((entry) => [
      seatLabel(entry.name, entry.partial),
      bar(entry.id, entry.tokens),
      compactCount(entry.tokens),
      `${entry.share}%`,
    ]);
    if (!all && view.other)
      lines.push([
        seatLabel(`Other (${view.other.seats} seats)`, false),
        // The folded seats together can outweigh the leader: their share says it, not a bar.
        '',
        compactCount(view.other.tokens),
        `${view.other.share}%`,
      ]);
    host.replaceChildren(table(['Seat', '', 'Tokens', 'Share'], lines));
  };
  draw(false);
  const body: Node[] = [host];
  if (view.partial)
    body.push(el('p', 'usage-note', '* Partial: some of this range has no data for the seat.'));
  if (view.other) {
    const toggle = el('button', ui.action);
    toggle.dataset.variant = 'text';
    toggle.type = 'button';
    toggle.setAttribute('aria-controls', host.id);
    const sync = (all: boolean) => {
      toggle.setAttribute('aria-expanded', String(all));
      toggle.textContent = all ? `Show top ${view.top.length}` : `Show all ${rows.length} seats`;
    };
    sync(false);
    toggle.addEventListener('click', () => {
      const all = toggle.getAttribute('aria-expanded') !== 'true';
      draw(all);
      sync(all);
    });
    body.push(toggle);
  }
  return card('digest-ranking', `Last ${doc.since} by seat`, body);
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
    const status = el('dd');
    status.append(passiveChip('Not connected'));
    line.append(el('dt', undefined, term), status, el('dd', 'digest-gap-source', source));
    list.append(line);
  }
  const method = el('details', ui.disclosure);
  method.dataset.divided = '';
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

/** What a report shows, without the read's own clock: a refresh that changed nothing repaints nothing. */
function fingerprint(report: DigestReport): string {
  return JSON.stringify({
    ...report,
    document: report.document && { ...report.document, observedAtMs: 0 },
  });
}

export function digestPage(source: DigestSource = digestSource, signal?: AbortSignal): void {
  const tabs = document.getElementById('usage-ranges');
  const panel = document.getElementById('digest-sections');
  const note = document.getElementById('usage-note');
  const badge = document.getElementById('usage-badge');
  if (!tabs || !panel || !note || !badge) return;
  const badgeLabel = badge.querySelector<HTMLElement>(`.${ui.chipLabel}`)!;
  let current: UsageRange = USAGE_RANGES[0];
  let generation = 0;
  let shown = '';
  // The last good report on screen, for the range being shown.
  let good: DigestReport | undefined;
  // The token chart's Absolute/Share choice survives a refresh.
  const scale = { share: false };
  const buttons = USAGE_RANGES.map((range) => {
    const button = el('button', ui.tab, range);
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
  function paint(report: DigestReport) {
    shown = fingerprint(report);
    good = undefined;
    badge!.hidden = true;
    badgeLabel.textContent = '';
    if (report.unavailable || !report.document) {
      panel!.replaceChildren();
      return notAvailable(note!, report.unavailable ?? 'Usage is not available.');
    }
    good = report;
    const doc = report.document;
    const activityBlock = activityCard(doc);
    panel!.replaceChildren(
      allowanceRow(doc),
      row('digest-row-burn', tokensCard(doc, scale), rankingCard(doc)),
      ...(activityBlock ? [activityBlock] : []),
      row('digest-row-coverage', exchangeCard(doc), coverageCard()),
    );
    if (report.sample) badgeLabel.textContent = 'Sample data';
    badge!.hidden = !report.sample;
    ordinaryNote(report);
  }
  /** The note a good report carries: the sample's preview line, else nothing. */
  function ordinaryNote(report: DigestReport) {
    if (report.sample) {
      notAvailable(note!, 'Usage preview. Real numbers: `tmt digest usage report`.');
    } else {
      note!.replaceChildren();
      note!.hidden = true;
    }
  }
  /**
   * A refresh that could not read keeps the last good report on screen and says why in a small
   * note; only a first read with nothing to show empties the panel. The next good read clears it.
   */
  function refreshed(report: DigestReport) {
    if (report.unavailable || !report.document) {
      if (!good?.document) return paint(report);
      const at = new Date(good.document.observedAtMs).toLocaleTimeString('en-GB', {
        hour: '2-digit',
        minute: '2-digit',
      });
      return notAvailable(
        note!,
        `${report.unavailable ?? 'Usage is not available.'} Showing the last report, read at ${at}.`,
      );
    }
    if (fingerprint(report) !== shown) return paint(report);
    // The same report as before: nothing to repaint, only a stale note to clear.
    ordinaryNote(report);
  }
  async function show(range: UsageRange) {
    current = range;
    const mine = ++generation;
    for (const button of buttons) {
      const selected = button.dataset.range === range;
      button.setAttribute('aria-selected', String(selected));
      button.tabIndex = selected ? 0 : -1;
    }
    panel!.replaceChildren(pageLoading());
    const report = await source.report(range, signal);
    if (mine !== generation || signal?.aborted) return;
    paint(report);
  }
  void show(current);
  // The usage read once a minute while the route is visible (the terminal Digest's cadence); the
  // page repaints only when the report changed, so a reader's place stays. The timer runs once at
  // once: that tick is the first read above, so it reads nothing.
  let first = true;
  if (signal)
    whileVisible(REFRESH_MS, signal, async () => {
      if (first) {
        first = false;
        return true;
      }
      const range = current;
      const mine = generation;
      const report = await source.report(range, signal);
      if (signal.aborted || range !== current || mine !== generation) return true;
      refreshed(report);
      return !report.unavailable;
    });
}
