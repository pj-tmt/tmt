import { htmlEl } from './chart-dom.js';
import { type MatrixOptions, matrixModel } from './chart-matrix-model.js';

export type { MatrixOptions, MatrixParticipant } from './chart-matrix-model.js';

/**
 * Who talks to whom: sender rows, recipient columns. A cell reads a number, `·` (observed none)
 * or `—` (unavailable); the diagonal is blank. Intensity is a square-root step of the busiest
 * pair, carried by class names that keep their text readable on both themes.
 */
export function createMatrix(options: MatrixOptions): HTMLElement {
  const figure = htmlEl('figure', 'tmt-ui-chart');
  figure.append(htmlEl('figcaption', 'tmt-ui-chart-title', options.title));
  if (!options.participants.length) {
    figure.append(htmlEl('p', 'tmt-ui-chart-empty', options.empty ?? 'No agents observed.'));
    return figure;
  }
  const model = matrixModel(options);
  const table = htmlEl('table', 'tmt-ui-chart-matrix');
  const head = htmlEl('tr');
  const corner = htmlEl('th', 'tmt-ui-chart-matrix-corner', 'from\\to');
  corner.scope = 'col';
  head.append(corner);
  for (const label of model.columns) {
    const column = htmlEl('th', 'tmt-ui-chart-matrix-column');
    column.scope = 'col';
    column.title = label;
    column.append(htmlEl('span', 'tmt-ui-chart-matrix-name', label));
    head.append(column);
  }
  const thead = htmlEl('thead');
  thead.append(head);
  const body = htmlEl('tbody');
  for (const row of model.rows) {
    const line = htmlEl('tr');
    const name = htmlEl('th', 'tmt-ui-chart-matrix-row');
    name.scope = 'row';
    name.title = row.label;
    name.append(htmlEl('span', 'tmt-ui-chart-matrix-name', row.label));
    line.append(name);
    for (const cell of row.cells) {
      const node = htmlEl('td', 'tmt-ui-chart-cell', cell.text);
      node.setAttribute('aria-label', cell.label);
      if (cell.self) node.classList.add('tmt-ui-chart-cell-self');
      if (cell.level) node.classList.add(`tmt-ui-chart-level-${cell.level}`);
      line.append(node);
    }
    if (row.sum !== undefined) {
      line.append(htmlEl('td', 'tmt-ui-chart-cell tmt-ui-chart-cell-sum', row.sum));
    }
    body.append(line);
  }
  table.append(thead, body);
  const scroll = htmlEl('div', 'tmt-ui-chart-scroll');
  scroll.tabIndex = 0;
  scroll.setAttribute('role', 'region');
  scroll.setAttribute('aria-label', options.title);
  scroll.append(table);
  const viewport = htmlEl('div', 'tmt-ui-chart-matrix-viewport');
  const edge = htmlEl('span', 'tmt-ui-chart-matrix-edge');
  edge.setAttribute('aria-hidden', 'true');
  edge.hidden = true;
  const updateEdge = () => {
    edge.hidden = scroll.scrollWidth - scroll.clientWidth - scroll.scrollLeft <= 1;
  };
  scroll.addEventListener('scroll', updateEdge, { passive: true });
  if (typeof ResizeObserver !== 'undefined') {
    const observer = new ResizeObserver(updateEdge);
    observer.observe(scroll);
    observer.observe(table);
  }
  viewport.append(scroll, edge);
  figure.append(viewport);
  return figure;
}
