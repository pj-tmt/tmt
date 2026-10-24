import { htmlEl } from './chart-dom.js';
import { type StripsOptions, STRIP_STEPS, stripsModel } from './chart-strips-model.js';

export type { StripRow, StripsOptions } from './chart-strips-model.js';

/** `Less ▢▢▢▢▢ More`: the swatches of the intensity steps, for a strip's caption or card header. */
export function stripsLegend(): HTMLElement {
  const legend = htmlEl('div', 'tmt-ui-chart-strip-legend');
  legend.setAttribute('aria-hidden', 'true');
  legend.append(htmlEl('span', undefined, 'Less'));
  for (let step = 0; step <= STRIP_STEPS; step += 1)
    legend.append(htmlEl('span', `tmt-ui-chart-slot tmt-ui-chart-slot-${step}`));
  legend.append(htmlEl('span', undefined, 'More'));
  return legend;
}

/**
 * Rows of fixed square slots, like a contribution graph: when each seat was active, scaled to
 * that row's busiest slot. The row is as long as its slot count; a slot's tip (hover or focus)
 * names the seat, the time range and the tokens.
 */
export function createStrips(options: StripsOptions): HTMLElement {
  const figure = htmlEl('figure', 'tmt-ui-chart');
  if (options.layout) figure.dataset.layout = options.layout;
  figure.append(htmlEl('figcaption', 'tmt-ui-chart-title', options.title));
  if (!options.rows.length) {
    figure.append(htmlEl('p', 'tmt-ui-chart-empty', options.empty ?? 'No agents observed.'));
    return figure;
  }
  const frame = htmlEl('div', 'tmt-ui-chart-frame');
  const list = htmlEl('ul', 'tmt-ui-chart-strips');
  list.style.setProperty('--tmt-chart-strip-count', String(options.slotCount));
  const tip = htmlEl('div', 'tmt-ui-chart-tip');
  tip.hidden = true;
  tip.setAttribute('aria-hidden', 'true');
  if (options.labels) {
    const dates = htmlEl('li', 'tmt-ui-chart-strip-row');
    dates.setAttribute('aria-hidden', 'true');
    const labels = htmlEl('div', 'tmt-ui-chart-strip tmt-ui-chart-strip-ends');
    for (const label of options.labels)
      labels.append(htmlEl('span', 'tmt-ui-chart-strip-date', label));
    dates.append(htmlEl('span', 'tmt-ui-chart-strip-label'), labels);
    list.append(dates);
  }
  const grid: { node: HTMLElement; tip: { seat: string; range: string; value: string } }[][] = [];
  for (const row of stripsModel(options).rows) {
    const item = htmlEl('li', 'tmt-ui-chart-strip-row');
    item.setAttribute('aria-label', row.summary);
    const name = htmlEl('span', 'tmt-ui-chart-strip-label', row.label);
    name.title = row.label;
    const strip = htmlEl('div', 'tmt-ui-chart-strip');
    const cells: (typeof grid)[number] = [];
    for (const slot of row.slots) {
      const node = htmlEl('span', 'tmt-ui-chart-slot');
      node.classList.add(
        slot.level === null ? 'tmt-ui-chart-slot-none' : `tmt-ui-chart-slot-${slot.level}`,
      );
      node.setAttribute('role', 'img');
      node.setAttribute('aria-label', slot.label);
      node.tabIndex = -1;
      strip.append(node);
      cells.push({ node, tip: slot.tip });
    }
    grid.push(cells);
    item.append(name, strip);
    list.append(item);
  }
  grid[0]![0]!.node.tabIndex = 0;
  const ends = htmlEl('li', 'tmt-ui-chart-strip-row');
  ends.setAttribute('aria-hidden', 'true');
  const span = htmlEl('div', 'tmt-ui-chart-strip-ends');
  span.append(
    htmlEl('span', undefined, options.ends[0]),
    htmlEl('span', undefined, options.ends[1]),
  );
  ends.append(htmlEl('span', 'tmt-ui-chart-strip-label'), span);
  if (options.layout !== 'heatmap') list.append(ends);

  const find = (target: EventTarget | null) => {
    const node = (target as HTMLElement | null)?.closest<HTMLElement>('.tmt-ui-chart-slot');
    if (!node) return undefined;
    for (const [r, cells] of grid.entries()) {
      const c = cells.findIndex((cell) => cell.node === node);
      if (c >= 0) return { r, c };
    }
    return undefined;
  };
  function show(r: number, c: number) {
    const cell = grid[r]![c]!;
    tip.replaceChildren(
      htmlEl('div', 'tmt-ui-chart-tip-title', cell.tip.seat),
      htmlEl('div', undefined, cell.tip.range),
      htmlEl('div', 'tmt-ui-chart-legend-value', cell.tip.value),
    );
    tip.hidden = false;
    const box = cell.node.getBoundingClientRect();
    const at = frame.getBoundingClientRect();
    const left = box.left - at.left + box.width / 2 - tip.offsetWidth / 2;
    tip.style.left = `${Math.max(0, Math.min(left, at.width - tip.offsetWidth))}px`;
    tip.style.top = `${box.bottom - at.top + 6}px`;
  }
  const hide = () => (tip.hidden = true);
  list.addEventListener('pointerover', (event) => {
    const at = find(event.target);
    if (at) show(at.r, at.c);
  });
  list.addEventListener('pointerleave', () => {
    const focused = find(document.activeElement);
    if (focused) show(focused.r, focused.c);
    else hide();
  });
  list.addEventListener('focusin', (event) => {
    const at = find(event.target);
    if (at) show(at.r, at.c);
  });
  list.addEventListener('focusout', hide);
  list.addEventListener('keydown', (event) => {
    const at = find(event.target);
    if (!at) return;
    const row = grid[at.r]!;
    const to =
      event.key === 'ArrowRight'
        ? { r: at.r, c: Math.min(row.length - 1, at.c + 1) }
        : event.key === 'ArrowLeft'
          ? { r: at.r, c: Math.max(0, at.c - 1) }
          : event.key === 'ArrowDown'
            ? { r: Math.min(grid.length - 1, at.r + 1), c: at.c }
            : event.key === 'ArrowUp'
              ? { r: Math.max(0, at.r - 1), c: at.c }
              : event.key === 'Home'
                ? { r: at.r, c: 0 }
                : event.key === 'End'
                  ? { r: at.r, c: row.length - 1 }
                  : undefined;
    if (!to) return;
    event.preventDefault();
    grid[at.r]![at.c]!.node.tabIndex = -1;
    const next = grid[to.r]![to.c]!.node;
    next.tabIndex = 0;
    next.focus();
  });
  frame.append(list, tip);

  figure.append(frame);
  if (options.legend !== false) figure.append(stripsLegend());
  return figure;
}
