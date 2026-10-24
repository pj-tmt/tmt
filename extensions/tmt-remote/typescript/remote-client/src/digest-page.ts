/**
 * The Digest usage report follows the concept website: provider allowance cards, the token
 * chart and seat legend, a full-width Messages heatmap, and a coverage/method footer. It depends
 * only on DigestSource; product data comes from the existing owner reads. Samples are injected
 * by tests, the gallery and local development. Range and view state belong to this mounted page.
 */
import { compactCount, createSeriesChart, topLayers } from '@tmt/browser-ui/chart';
import { pageLoading } from './page-loading.js';
import {
  createDigestSource,
  USAGE_RANGES,
  type DigestReport,
  type DigestSource,
  type UsageDocument,
  type UsageRange,
} from './digest-source.js';
import {
  exchange,
  providerGroups,
  observedAgo,
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
const SERIES_KEPT = 6;
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

type SeatOrder = 'usage' | 'name';
const SEAT_ORDER_KEY = 'tmt-digest-seat-order';

function readSeatOrder(): SeatOrder {
  try {
    return localStorage.getItem(SEAT_ORDER_KEY) === 'name' ? 'name' : 'usage';
  } catch {
    return 'usage';
  }
}

function writeSeatOrder(value: SeatOrder): SeatOrder {
  try {
    localStorage.setItem(SEAT_ORDER_KEY, value);
    return value;
  } catch {
    return 'usage';
  }
}

interface DigestViewState {
  share: boolean;
  allSeats: boolean;
  lines: boolean;
  order: SeatOrder;
  dispose: (() => void)[];
}

/** A flat sheet with a sentence-case title and an optional right-hand slot in its head. */
function card(
  id: string,
  title: string,
  body: readonly Node[],
  options: { actions?: Node; className?: string; subtitle?: string } = {},
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
  if (options.subtitle) {
    const title = el('div', 'digest-section-title');
    title.append(heading, el('p', 'digest-subtitle', options.subtitle));
    head.append(title);
  } else head.append(heading);
  if (options.actions) head.append(options.actions);
  node.append(head, ...body);
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
    button.id = `${controls}-${option.value}`;
    button.addEventListener('keydown', (event) => {
      const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
      if (!step) return;
      event.preventDefault();
      const index = buttons.findIndex((item) => item.value === option.value);
      const next = buttons[(index + step + buttons.length) % buttons.length]!;
      next.button.click();
      next.button.focus();
    });
    button.addEventListener('click', () => {
      sync(option.value);
      change(option.value);
    });
    return { button, value: option.value };
  });
  const sync = (active: T) => {
    for (const { button, value } of buttons) {
      button.setAttribute('aria-selected', String(value === active));
      button.tabIndex = value === active ? 0 : -1;
    }
  };
  sync(initial);
  list.append(...buttons.map(({ button }) => button));
  header.append(list);
  return header;
}

/** One numeric fact inside a provider's history disclosure. */
function tile(label: string, value: string): HTMLElement {
  const node = el('div', ui.statusTile);
  const figure = el('dd', ui.statusTileValue, value);
  figure.dataset.numeric = value === '0' ? 'zero' : 'count';
  node.append(el('dt', ui.statusTileLabel, label), figure);
  return node;
}

/** A group of bare tiles for use inside a card. */
function bareTiles(...tiles: HTMLElement[]): HTMLElement {
  const group = el('dl', ui.statusTiles);
  group.dataset.bare = '';
  group.append(...tiles);
  return group;
}

function providerRow(view: ProviderView, nowMs: number): HTMLElement {
  const row = el('div', 'digest-provider');
  row.id = `digest-window-${view.key}`;
  const label = el('div', 'digest-provider-label');
  label.append(
    el('strong', undefined, view.provider),
    el('strong', undefined, `${view.leftPercent}% left${view.stale ? ' (stale)' : ''}`),
  );
  const meter = el('div', ui.meter);
  meter.setAttribute('role', 'img');
  meter.setAttribute('aria-label', `${view.leftPercent}% of the weekly allowance left`);
  const fill = el('div', `${ui.meterFill} tmt-ui-chart-s${view.style}`);
  fill.style.setProperty(
    '--tmt-ui-meter-fill',
    String(Math.max(0, Math.min(100, view.leftPercent))),
  );
  meter.append(fill);
  const details = el('details', `${ui.disclosure} digest-provider-details`);
  details.id = `${row.id}-history`;
  const summary = el('summary', undefined, 'Weekly history and pace');
  summary.id = `${details.id}-summary`;
  details.append(summary);
  details.append(
    view.chart
      ? createSeriesChart(view.chart)
      : el(
          'p',
          'usage-note',
          'The weekly history is not served yet; this shows the latest window.',
        ),
  );
  details.append(
    bareTiles(
      tile('Elapsed', view.elapsedPercent === null ? '—' : `${Math.round(view.elapsedPercent)}%`),
      tile(
        'Pace',
        view.paceDeltaPoints === null
          ? '—'
          : `${view.paceDeltaPoints > 0 ? '+' : ''}${view.paceDeltaPoints.toFixed(1)} pts`,
      ),
      tile('Resets in', view.stale ? 'stale' : resetsIn(view.resetsAtMs, nowMs)),
    ),
  );
  const observation = el(
    'p',
    'usage-note digest-window-observation',
    `Observed ${observedAgo(view.observedAtMs, nowMs)}`,
  );
  observation.dataset.observedAt = String(view.observedAtMs);
  const reset = new Date(view.resetsAtMs).toLocaleString('en-GB', {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    timeZoneName: 'short',
  });
  const metadata = el('div', 'digest-provider-metadata');
  metadata.append(
    observation,
    el('p', 'usage-note', `Resets ${reset}`),
    el(
      'p',
      'usage-note digest-reporters',
      `Reported by ${view.reporters.join(', ') || 'unknown seats'}`,
    ),
  );
  row.append(label, meter, metadata, details);
  return row;
}

function allowanceRow(report: DigestReport, state: DigestViewState): HTMLElement {
  const doc = report.allowance;
  const intro = el('div', 'digest-allowance-intro');
  intro.append(
    el('h2', ui.cardTitle, 'Allowance'),
    el('p', 'digest-subtitle', 'Latest known weekly windows. Independent of the usage range.'),
  );
  const node = el('section', `${ui.card} digest-card digest-allowance`);
  node.id = 'digest-allowance';
  intro.querySelector('h2')!.id = 'digest-allowance-title';
  node.setAttribute('aria-labelledby', 'digest-allowance-title');
  const groups = el('div', 'digest-provider-groups');
  groups.id = 'digest-provider-groups';
  groups.setAttribute('role', 'region');
  groups.setAttribute('aria-label', 'Provider allowances');
  const providers = providerGroups(doc);
  if (providers.length > 2) groups.dataset.many = '';
  for (const group of providers) {
    const host = el('div', 'digest-provider-group');
    host.id = `digest-provider-${encodeURIComponent(group.source)}`;
    if (group.primary) host.append(providerRow(group.primary, doc!.observedAtMs));
    else {
      const label = el('div', 'digest-provider-label');
      label.append(el('strong', undefined, group.label), el('span', undefined, '○ not reported'));
      host.append(
        label,
        el(
          'p',
          'usage-note',
          group.source === 'claude'
            ? 'Claude Code does not report its allowance to TMT.'
            : (report.allowanceUnavailable ??
                'No current seat has reported an active weekly window.'),
        ),
      );
    }
    if (group.other.length) {
      const other = el('details', `${ui.disclosure} digest-other-windows`);
      other.id = `${host.id}-other`;
      const summary = el(
        'summary',
        undefined,
        `Other ${group.label} windows seen this week (${group.other.length})`,
      );
      summary.id = `${other.id}-summary`;
      other.append(summary, ...group.other.map((view) => providerRow(view, doc!.observedAtMs)));
      host.append(other);
    }
    const symbol = el(
      'span',
      'digest-provider-symbol',
      group.source === 'claude' ? '✳' : group.source === 'codex' ? '⌘' : '○',
    );
    symbol.setAttribute('aria-hidden', 'true');
    host.querySelector('.digest-provider-label')?.prepend(symbol);
    groups.append(host);
  }
  const controls = el('div', 'digest-carousel-controls');
  const previous = el('button', ui.action, '←');
  const next = el('button', ui.action, '→');
  for (const [button, name, direction] of [
    [previous, 'Previous provider', -1],
    [next, 'Next provider', 1],
  ] as const) {
    button.type = 'button';
    button.dataset.variant = 'secondary';
    button.setAttribute('aria-label', name);
    button.setAttribute('aria-controls', groups.id);
    button.addEventListener('click', () => {
      const first = groups.firstElementChild;
      const step =
        (first?.getBoundingClientRect().width ?? groups.clientWidth) +
        Number.parseFloat(getComputedStyle(groups).columnGap);
      groups.scrollBy({ left: direction * step, behavior: 'smooth' });
    });
  }
  const sync = () => {
    const overflow = groups.scrollWidth > groups.clientWidth + 1;
    controls.hidden = !overflow;
    groups.tabIndex = overflow ? 0 : -1;
    previous.disabled = groups.scrollLeft <= 1;
    next.disabled = groups.scrollLeft + groups.clientWidth >= groups.scrollWidth - 1;
  };
  groups.addEventListener('scroll', sync);
  const observer = new ResizeObserver(sync);
  observer.observe(groups);
  state.dispose.push(() => {
    observer.disconnect();
    groups.removeEventListener('scroll', sync);
  });
  controls.append(previous, next);
  intro.append(controls);
  node.append(intro, groups);
  if (report.allowanceUnavailable) node.append(el('p', 'usage-note', report.allowanceUnavailable));
  else if (doc?.limits.some((limit) => !limit.provider))
    node.append(
      el(
        'p',
        'usage-note',
        'Some allowance windows have no reported provider. Upgrade TMT on your machine to identify them.',
      ),
    );
  return node;
}

function tokensCard(doc: UsageDocument, state: DigestViewState): HTMLElement {
  const data = tokenSeries(doc);
  const body = el('div', 'digest-token-grid tmt-ui-chart-seats');
  const chart = el('div', 'digest-token-chart');
  const rows = rankRows(doc);
  const total = rows.reduce((sum, row) => sum + row.tokens, 0);
  chart.append(
    el(
      'p',
      'digest-total-label',
      `Total${data.partial || doc.usage.unavailable.length > 0 || rows.some((row) => row.partial) ? ' (lower bound)' : ''} · ${doc.since}`,
    ),
    el('p', 'digest-total', rows.length ? compactCount(total) : '—'),
  );
  const host = el('div');
  const draw = () => {
    const layers = state.lines
      ? topLayers(data.layers, data.layers.length)
      : topLayers(data.layers, SERIES_KEPT);
    const series =
      state.lines && state.share
        ? layers.map((layer) => ({
            ...layer,
            values: layer.values.map((value, at) => {
              if (value === null) return null;
              const total = layers.reduce((sum, item) => sum + (item.values[at] ?? 0), 0);
              return total ? (value / total) * 100 : 0;
            }),
          }))
        : layers;
    host.setAttribute(
      'aria-labelledby',
      `digest-tokens-chart-${state.share ? 'share' : 'absolute'}`,
    );
    host.replaceChildren(
      createSeriesChart({
        title: `${state.share ? 'Share of tokens' : 'Tokens'} by seat${state.lines ? ' · Lines' : ''}`,
        kind: state.lines ? 'line' : state.share ? 'share' : 'stack',
        curve: 'monotone',
        x: data.x,
        series,
        ...(state.lines && state.share
          ? { max: 100, format: (value: number) => `${Math.round(value)}%` }
          : {}),
        empty: 'No token observations in this range.',
      }),
    );
    const plot = host.querySelector<HTMLElement>('.tmt-ui-chart-plot');
    if (plot) plot.id = 'digest-token-plot';
  };
  draw();
  chart.append(host);
  let legend = legendTable(doc, state);
  body.append(chart, legend);
  if (data.partial)
    chart.append(
      el('p', 'usage-note', 'Some buckets are incomplete: they are lower bounds, not errors.'),
    );
  if (doc.usage.unavailable.length)
    chart.append(el('p', 'usage-note', 'Unavailable seats are not included in this total.'));
  host.id = 'digest-tokens-chart';
  host.setAttribute('role', 'tabpanel');
  host.setAttribute('aria-label', 'Token usage scale');
  const control = tabChoice(
    'Scale',
    host.id,
    [
      { value: 'absolute', text: 'Absolute' },
      { value: 'share', text: 'Share' },
    ],
    state.share ? 'share' : 'absolute',
    (value) => {
      state.share = value === 'share';
      draw();
    },
  );
  const style = tabChoice(
    'Chart style',
    host.id,
    [
      { value: 'stacked', text: 'Stacked' },
      { value: 'lines', text: 'Lines' },
    ],
    state.lines ? 'lines' : 'stacked',
    (value) => {
      state.lines = value === 'lines';
      draw();
      const next = legendTable(doc, state);
      legend.replaceWith(next);
      legend = next;
    },
  );
  const controls = el('div', 'digest-chart-controls');
  controls.append(control, style);
  return card('digest-tokens', 'Token usage', [body], {
    actions: controls,
    subtitle: 'A shared view, seat by seat.',
  });
}

/** A seat's name on one line, the full name on hover, `*` after a partial one. */
function seatLabel(name: string, partial: boolean): HTMLElement {
  const node = el('span', 'digest-seat', name);
  node.title = partial ? `${name} (partial)` : name;
  if (partial) node.append(el('span', 'digest-partial', '*'));
  return node;
}

function legendTable(doc: UsageDocument, state: DigestViewState): HTMLElement {
  const ranked = rankRows(doc);
  const rows = [...ranked].sort(
    (a, b) =>
      (state.order === 'usage' ? b.tokens - a.tokens : 0) ||
      a.name.localeCompare(b.name) ||
      a.id.localeCompare(b.id),
  );
  const view = ranking(ranked, SERIES_KEPT);
  const chartLayers = state.lines
    ? topLayers(tokenSeries(doc).layers, tokenSeries(doc).layers.length)
    : topLayers(tokenSeries(doc).layers, SERIES_KEPT);
  // The seats the token chart draws as hues keep that hue here; the rest share the neutral step.
  const hue = new Map(
    chartLayers.filter((layer) => layer.key !== 'other').map((layer, at) => [layer.key, at % 7]),
  );
  function legendSeat(id: string, name: string, partial: boolean): HTMLElement {
    const label = el('span', 'digest-legend-seat');
    const dot = el('span', `tmt-ui-status-tile-dot tmt-ui-chart-s${hue.get(id) ?? OTHER_STEP}`);
    dot.setAttribute('aria-hidden', 'true');
    label.append(dot, seatLabel(name, partial));
    return label;
  }
  const host = el('div', 'digest-ranking-table');
  host.id = 'digest-ranking-rows';
  const draw = (all: boolean) => {
    const listed =
      all || state.lines ? rows : rows.filter((row) => view.top.some((top) => top.id === row.id));
    const lines: (string | Node)[][] = listed.map((entry) => [
      legendSeat(entry.id, entry.name, entry.partial),

      compactCount(entry.tokens),
      `${entry.share}%`,
    ]);
    if (!all && !state.lines && view.other)
      lines.push([
        legendSeat('other', `Other (${view.other.seats} seats)`, false),
        compactCount(view.other.tokens),
        `${view.other.share}%`,
      ]);
    host.replaceChildren(table(['Seat', 'Tokens', 'Share'], lines));
  };
  draw(state.allSeats);
  const body: Node[] = [host];
  if (view.partial)
    body.push(el('p', 'usage-note', '* Partial: some of this range has no data for the seat.'));
  if (view.other && !state.lines) {
    const toggle = el('button', ui.action);
    toggle.dataset.variant = 'text';
    toggle.type = 'button';
    toggle.id = 'digest-ranking-toggle';
    toggle.setAttribute('aria-controls', host.id);
    const sync = (all: boolean) => {
      toggle.setAttribute('aria-expanded', String(all));
      toggle.textContent = all ? `Show top ${view.top.length}` : `Show all ${rows.length} seats`;
    };
    sync(state.allSeats);
    toggle.addEventListener('click', () => {
      const all = toggle.getAttribute('aria-expanded') !== 'true';
      state.allSeats = all;
      draw(all);
      sync(all);
    });
    body.push(toggle);
  }
  const legend = el('div', 'digest-legend');
  legend.id = 'digest-ranking';
  legend.append(...body);
  return legend;
}

/** This one caller owns its count heatmap; report aggregation remains in exchange(). */
function exchangeCard(doc: UsageDocument, state: DigestViewState): HTMLElement {
  const flow = exchange(doc);
  if (!flow)
    return card('digest-exchange', 'Messages between seats', [
      el('p', 'usage-note', 'Message traffic is not available in this read.'),
    ]);
  const totals = new Map(rankRows(doc).map((seat) => [seat.id, seat.tokens]));
  const seats = [...flow.participants].sort(
    (a, b) =>
      (state.order === 'usage' ? (totals.get(b.id) ?? 0) - (totals.get(a.id) ?? 0) : 0) ||
      a.label.localeCompare(b.label) ||
      a.id.localeCompare(b.id),
  );
  const maximum = Math.max(
    0,
    ...seats.flatMap((from) =>
      seats.map((to) => (from.id === to.id ? 0 : (flow.value(from.id, to.id) ?? 0))),
    ),
  );
  const matrix = el('table', 'digest-messages');
  matrix.style.setProperty('--digest-message-columns', String(seats.length + 1));
  if (seats.length > 6) matrix.dataset.large = '';
  const caption = el('caption', 'digest-visually-hidden', `Messages, last ${doc.since}`);
  const header = el('tr');
  const corner = el('th', undefined, 'From →');
  corner.scope = 'col';
  header.append(corner);
  for (const seat of seats) {
    const cell = el('th');
    cell.scope = 'col';
    cell.dataset.seatId = seat.id;
    cell.append(messageName(seat.label));
    header.append(cell);
  }
  const head = el('thead');
  head.append(header);
  const body = el('tbody');
  for (const from of seats) {
    const row = el('tr');
    const label = el('th');
    label.scope = 'row';
    label.dataset.seatId = from.id;
    label.append(messageName(from.label));
    row.append(label);
    for (const to of seats) {
      const self = from.id === to.id;
      const count = self ? null : flow.value(from.id, to.id);
      const cell = el('td', undefined, count === null ? '—' : compactCount(count));
      cell.dataset.from = from.id;
      cell.dataset.to = to.id;
      if (self) cell.dataset.self = '';
      else if (count === null) cell.dataset.missing = '';
      else if (count > 0)
        cell.dataset.level = String(Math.min(4, Math.ceil((count / maximum) * 4)));
      const text = self ? 'same seat' : count === null ? 'not observed' : `${count} messages`;
      cell.setAttribute('aria-label', `${from.label} to ${to.label}: ${text}`);
      cell.title = `${from.label} to ${to.label}: ${text}`;
      row.append(cell);
    }
    body.append(row);
  }
  matrix.append(caption, head, body);
  const viewport = el('div', 'digest-messages-viewport');
  const scroll = el('div', 'digest-messages-scroll');
  scroll.id = 'digest-exchange-scroll';
  scroll.setAttribute('role', 'region');
  scroll.setAttribute('aria-label', `Messages, last ${doc.since}`);
  scroll.append(matrix);
  const edge = el('span', 'digest-messages-edge');
  edge.setAttribute('aria-hidden', 'true');
  const sync = () => {
    const overflow = scroll.scrollWidth > scroll.clientWidth + 1;
    scroll.tabIndex = overflow ? 0 : -1;
    edge.hidden = !overflow || scroll.scrollLeft + scroll.clientWidth >= scroll.scrollWidth - 1;
  };
  scroll.addEventListener('scroll', sync);
  const observer = new ResizeObserver(sync);
  observer.observe(scroll);
  state.dispose.push(() => {
    observer.disconnect();
    scroll.removeEventListener('scroll', sync);
  });
  viewport.append(scroll, edge);
  return card('digest-exchange', 'Messages between seats', [
    el('p', 'digest-subtitle', `Talk and reply messages · ${doc.since}`),
    viewport,
  ]);
}

function messageName(text: string): HTMLElement {
  const label = el('span', 'digest-message-name', text);
  label.title = text;
  return label;
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
    status.append(el('span', 'digest-subtitle', 'Not connected'));
    const name = el('dt', undefined, term === 'Machine' ? 'Machine load' : term);
    const marker = el('span', 'digest-coverage-marker');
    marker.setAttribute('aria-hidden', 'true');
    name.prepend(marker);
    line.append(name, status);
    line.title = source;
    list.append(line);
  }
  const method = el('details', ui.disclosure);
  method.id = 'digest-method';
  const summary = el('summary', undefined, 'Method & caveats');
  summary.id = 'digest-method-summary';
  method.append(
    summary,
    ...[
      'Tokens are input plus output as Core reports them; cached input is part of input and is not counted again.',
      'An incomplete bucket is a lower bound. A seat with no evidence for part of the range is marked partial, never shown as zero.',
      'Allowance windows are the account percent as the provider reports it, in whole percent; usage on other machines is not seen.',
      'Message counts include talk and reply only.',
    ].map((text) => el('p', 'usage-note', text)),
  );
  const command = el('div', 'digest-method-command');
  command.append(
    el('span', undefined, 'Current data on your machine:'),
    el('code', undefined, 'tmt digest usage report'),
  );
  method.append(command);
  return card('digest-missing', 'Data coverage', [list, method]);
}

/** What a report shows, without the read's own clock: a refresh that changed nothing repaints nothing. */
function fingerprint(report: DigestReport): string {
  return JSON.stringify({
    ...report,
    document: report.document && { ...report.document, observedAtMs: 0 },
    allowance: report.allowance && {
      ...report.allowance,
      observedAtMs: 0,
      limits: report.allowance.limits.filter(
        (limit) => limit.resetsAtMs > report.allowance!.observedAtMs,
      ),
    },
  });
}

export function digestPage(
  source: DigestSource = createDigestSource(),
  signal?: AbortSignal,
): void {
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
  const state: DigestViewState = {
    share: false,
    allSeats: false,
    lines: false,
    order: readSeatOrder(),
    dispose: [],
  };
  signal?.addEventListener(
    'abort',
    () => {
      state.dispose.splice(0).forEach((dispose) => dispose());
    },
    { once: true },
  );
  const sort = el('div', 'digest-seat-order');
  const renderSort = () => {
    sort.replaceChildren(
      el('span', 'digest-sort-label', 'Seat order'),
      tabChoice(
        'Seat order',
        'usage-panel',
        [
          { value: 'usage', text: 'Usage' },
          { value: 'name', text: 'Name' },
        ],
        state.order,
        (value) => {
          state.order = writeSeatOrder(value);
          if (good?.range === current) paint(good, false);
          renderSort();
          document.getElementById(`usage-panel-${state.order}`)?.focus({ preventScroll: true });
        },
      ),
    );
  };
  renderSort();
  tabs.closest('.digest-controls')?.append(sort);
  const disclosures = new Map<string, boolean>();
  let matrixLeft = 0;
  let providerLeft = 0;
  function rememberView() {
    for (const detail of panel!.querySelectorAll('details[id]'))
      disclosures.set(detail.id, detail.hasAttribute('open'));
    matrixLeft = document.getElementById('digest-exchange-scroll')?.scrollLeft ?? matrixLeft;
    providerLeft = document.getElementById('digest-provider-groups')?.scrollLeft ?? providerLeft;
  }
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
      void show(range);
      buttons[USAGE_RANGES.indexOf(range)]!.focus();
    });
    return button;
  });
  tabs.replaceChildren(...buttons);
  function paint(report: DigestReport, updateNotice = true) {
    rememberView();
    const focusId = panel!.contains(document.activeElement)
      ? document.activeElement?.id
      : undefined;
    shown = fingerprint(report);
    good = undefined;
    badge!.hidden = true;
    badgeLabel.textContent = '';
    if (report.unavailable || !report.document) {
      state.dispose.splice(0).forEach((dispose) => dispose());
      panel!.replaceChildren();
      return notAvailable(note!, report.unavailable ?? 'Usage is not available.');
    }
    good = report;
    const doc = report.document;
    state.dispose.splice(0).forEach((dispose) => dispose());
    panel!.replaceChildren(
      allowanceRow(report, state),
      tokensCard(doc, state),
      exchangeCard(doc, state),
      coverageCard(),
    );
    for (const detail of panel!.querySelectorAll<HTMLDetailsElement>('details[id]'))
      detail.open = disclosures.get(detail.id) ?? false;
    const scroll = document.getElementById('digest-exchange-scroll');
    if (scroll) scroll.scrollLeft = matrixLeft;
    const providers = document.getElementById('digest-provider-groups');
    if (providers) providers.scrollLeft = providerLeft;
    if (focusId) document.getElementById(focusId)?.focus({ preventScroll: true });
    if (report.sample) badgeLabel.textContent = 'Sample data';
    badge!.hidden = !report.sample;
    if (updateNotice) ordinaryNote(report);
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
    for (const observation of panel!.querySelectorAll<HTMLElement>('.digest-window-observation'))
      observation.textContent = `Observed ${observedAgo(Number(observation.dataset.observedAt), report.allowance!.observedAtMs)}`;
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
    rememberView();
    state.dispose.splice(0).forEach((dispose) => dispose());
    panel!.replaceChildren(pageLoading());
    const report = await source.report(range, signal);
    if (mine !== generation || signal?.aborted) return;
    paint(report);
  }
  void show(current);
  // Refresh at the terminal Digest's cadence while mounted and visible. The initial read
  // above already covers the first visible tick; unchanged reports keep the existing DOM.
  let first = true;
  if (signal)
    whileVisible(REFRESH_MS, signal, async () => {
      if (first) {
        first = false;
        return true;
      }
      const range = current;
      const mine = generation;
      const report = await source.report(range, signal, true);
      if (signal.aborted || range !== current || mine !== generation) return true;
      refreshed(report);
      return !report.unavailable;
    });
}
