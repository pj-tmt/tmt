/** The Digest web view: one range picker and the per-agent table (tokens, share, turns). */
import {
  digestSource,
  USAGE_RANGES,
  type DigestSource,
  type UsageRange,
  type UsageReport,
} from './digest-source.js';

/** 1.9B, 48.2M, 310K, 940: compact token counts in the table. */
export function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000_000) return `${(tokens / 1_000_000_000).toFixed(1)}B`;
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1_000) return `${Math.round(tokens / 1_000)}K`;
  return String(tokens);
}

/** Each agent's share of the range's tokens, in whole percent, largest first. */
export function usageRows(report: UsageReport) {
  const total = report.agents.reduce((sum, row) => sum + row.tokens, 0);
  return [...report.agents]
    .sort((a, b) => b.tokens - a.tokens || a.agent.localeCompare(b.agent))
    .map((row) => ({
      ...row,
      share: total ? Math.round((row.tokens / total) * 100) : 0,
    }));
}

function cell(tag: 'td' | 'th', text: string): HTMLTableCellElement {
  const node = document.createElement(tag);
  node.textContent = text;
  return node;
}

export function digestPage(source: DigestSource = digestSource): void {
  const tabs = document.getElementById('usage-ranges');
  const body = document.getElementById('usage-rows');
  const note = document.getElementById('usage-note');
  if (!tabs || !body || !note) return;
  let current: UsageRange = '7d';
  let generation = 0;
  const buttons = USAGE_RANGES.map((range) => {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'tmt-ui-tab';
    button.setAttribute('role', 'tab');
    button.setAttribute('aria-controls', 'usage-panel');
    button.dataset.range = range;
    const label = document.createElement('span');
    label.className = 'tmt-ui-tab-label';
    label.textContent = range;
    button.append(label);
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
    const report = await source.usage(range);
    if (mine !== generation) return;
    body!.replaceChildren(
      ...usageRows(report).map((row) => {
        const line = document.createElement('tr');
        line.append(
          cell('th', row.agent),
          cell('td', formatTokens(row.tokens)),
          cell('td', `${row.share}%`),
          cell('td', row.turns === undefined ? '—' : String(row.turns)),
        );
        line.firstElementChild!.setAttribute('scope', 'row');
        return line;
      }),
    );
    note!.textContent =
      report.unavailable ??
      (report.sample
        ? 'Sample data: Digest has no web read of your usage yet. `tmt digest usage report` shows the real numbers.'
        : report.agents.some((row) => row.turns === undefined)
          ? 'Turns show — where Digest does not count them yet.'
          : '');
    note!.hidden = !note!.textContent;
  }
  void show(current);
}
