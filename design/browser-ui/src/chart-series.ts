import { dataTable, htmlEl, paintAtWidth, svgEl } from './chart-dom.js';
import { type SeriesChartOptions, seriesModel } from './chart-series-model.js';

export type { SeriesChartOptions } from './chart-series-model.js';

/** A time-series chart: stacked areas, a 100 % share, columns or lines on one axis. */
export function createSeriesChart(options: SeriesChartOptions): HTMLElement {
  const figure = htmlEl('figure', 'tmt-ui-chart');
  figure.setAttribute('role', 'group');
  figure.setAttribute('aria-label', options.title);
  figure.append(htmlEl('figcaption', 'tmt-ui-chart-title', options.title));
  const observed = options.series.some((layer) => layer.values.some((v) => v != null));
  if (!observed || !options.x.length) {
    figure.append(htmlEl('p', 'tmt-ui-chart-empty', options.empty ?? 'No observations yet.'));
    return figure;
  }
  const plot = svgEl('svg', {}, 'tmt-ui-chart-plot');
  plot.setAttribute('role', 'img');
  plot.setAttribute('tabindex', '0');
  plot.setAttribute('aria-label', `${options.title}. Arrow keys read each position.`);
  const readout = htmlEl('p', 'tmt-ui-chart-readout');
  readout.setAttribute('aria-live', 'polite');
  const frame = htmlEl('div', 'tmt-ui-chart-frame');
  const tip = htmlEl('div', 'tmt-ui-chart-tip');
  tip.hidden = true;
  tip.setAttribute('aria-hidden', 'true');
  const legend = htmlEl('ul', 'tmt-ui-chart-legend');
  const first = seriesModel(options, 640);
  const valueNodes = first.legend.map(({ label, style }) => {
    const item = htmlEl('li', 'tmt-ui-chart-legend-item');
    const swatch = htmlEl('span', `tmt-ui-chart-swatch tmt-ui-chart-s${style}`);
    swatch.setAttribute('aria-hidden', 'true');
    const value = htmlEl('span', 'tmt-ui-chart-legend-value');
    item.append(swatch, htmlEl('span', 'tmt-ui-chart-legend-label', label), value);
    legend.append(item);
    return value;
  });
  if (options.reference) {
    const item = htmlEl('li', 'tmt-ui-chart-legend-item');
    const swatch = htmlEl('span', 'tmt-ui-chart-swatch tmt-ui-chart-swatch-reference');
    swatch.setAttribute('aria-hidden', 'true');
    item.append(swatch, htmlEl('span', 'tmt-ui-chart-legend-label', options.reference.label));
    legend.append(item);
  }
  let model = first;
  let active = options.x.length - 1;
  /** The hover tip beside the column, flipped to the left half's other side near the right edge. */
  function showTip(at: number) {
    const data = model.tip(at);
    const title = htmlEl('div', 'tmt-ui-chart-tip-title', data.title);
    const rows = data.rows.map((row) => {
      const line = htmlEl('div', 'tmt-ui-chart-tip-row');
      const swatch = htmlEl('span', `tmt-ui-chart-swatch tmt-ui-chart-s${row.style}`);
      line.append(
        swatch,
        htmlEl('span', 'tmt-ui-chart-tip-label', row.label),
        htmlEl('span', 'tmt-ui-chart-legend-value', row.value),
      );
      return line;
    });
    tip.replaceChildren(title, ...rows);
    tip.hidden = false;
    const x = model.xs[at] ?? 0;
    const flip = x > model.width / 2;
    tip.classList.toggle('tmt-ui-chart-tip-left', flip);
    tip.style.left = `${flip ? x - 12 : x + 12}px`;
  }
  function select(at: number) {
    active = Math.max(0, Math.min(options.x.length - 1, at));
    model.values(active).forEach((text, i) => (valueNodes[i]!.textContent = text));
    readout.textContent = model.readout(active);
    const cursor = plot.querySelector('.tmt-ui-chart-cursor');
    const x = String(model.xs[active] ?? 0);
    cursor?.setAttribute('x1', x);
    cursor?.setAttribute('x2', x);
    if (!tip.hidden) showTip(active);
  }
  function paint(width: number) {
    model = seriesModel(options, width);
    plot.replaceChildren();
    plot.setAttribute('width', String(model.width));
    plot.setAttribute('height', String(model.height));
    plot.setAttribute('viewBox', `0 0 ${model.width} ${model.height}`);
    const text = (x: number, y: number, label: string, anchor?: string) => {
      const node = svgEl('text', { x, y }, 'tmt-ui-chart-tick');
      if (anchor) node.setAttribute('text-anchor', anchor);
      node.textContent = label;
      plot.append(node);
    };
    for (const line of model.grid) {
      plot.append(
        svgEl('line', { x1: line.x1, x2: line.x2, y1: line.y, y2: line.y }, 'tmt-ui-chart-grid'),
      );
      text(line.x1 - 6, line.y + 4, line.label, 'end');
    }
    for (const label of model.xLabels) text(label.x, model.height - 6, label.text, label.anchor);
    for (const mark of model.marks) {
      if (mark.kind === 'bar') {
        plot.append(
          svgEl('rect', { ...mark.rect! }, `tmt-ui-chart-bar tmt-ui-chart-s${mark.style}`),
        );
      } else {
        plot.append(
          svgEl('path', { d: mark.d! }, `tmt-ui-chart-${mark.kind} tmt-ui-chart-s${mark.style}`),
        );
      }
    }
    if (model.reference)
      plot.append(svgEl('path', { d: model.reference }, 'tmt-ui-chart-reference'));
    for (const guide of model.guides) {
      plot.append(
        svgEl(
          'line',
          { x1: guide.x, x2: guide.x, y1: guide.top, y2: guide.bottom },
          'tmt-ui-chart-guide',
        ),
      );
      text(
        guide.anchor === 'end' ? guide.x - 4 : guide.x + 4,
        guide.top + 10,
        guide.label,
        guide.anchor,
      );
    }
    plot.append(
      svgEl('line', { x1: 0, x2: 0, y1: model.top, y2: model.bottom }, 'tmt-ui-chart-cursor'),
    );
    select(active);
  }
  plot.addEventListener('keydown', (event) => {
    const to =
      event.key === 'ArrowLeft'
        ? active - 1
        : event.key === 'ArrowRight'
          ? active + 1
          : event.key === 'Home'
            ? 0
            : event.key === 'End'
              ? options.x.length - 1
              : undefined;
    if (to === undefined) return;
    event.preventDefault();
    select(to);
  });
  const pointer = (event: PointerEvent) => {
    if (!model.step) return;
    select(
      Math.round((event.clientX - plot.getBoundingClientRect().left - model.left) / model.step),
    );
  };
  const point = (event: PointerEvent) => {
    pointer(event);
    showTip(active);
  };
  plot.addEventListener('pointermove', point);
  plot.addEventListener('pointerdown', point);
  plot.addEventListener('pointerleave', () => (tip.hidden = true));
  plot.addEventListener('focus', () => showTip(active));
  plot.addEventListener('blur', () => (tip.hidden = true));
  frame.append(plot, tip);
  figure.append(
    frame,
    readout,
    legend,
    dataTable(options.title, first.table.columns, first.table.rows),
  );
  paintAtWidth(figure, 640, paint);
  return figure;
}
