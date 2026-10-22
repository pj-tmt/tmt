/**
 * DOM helpers the leaf's chart painters share. No style attributes in markup: every look comes from a
 * `tmt-ui-chart-*` class, so a chart works under a strict style policy.
 */
const SVG = 'http://www.w3.org/2000/svg';

export function svgEl<K extends keyof SVGElementTagNameMap>(
  name: K,
  attrs: Record<string, string | number> = {},
  className?: string,
): SVGElementTagNameMap[K] {
  const node = document.createElementNS(SVG, name);
  for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, String(value));
  if (className) node.setAttribute('class', className);
  return node;
}

export function htmlEl<K extends keyof HTMLElementTagNameMap>(
  name: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(name);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/**
 * Call `paint(width)` once now and again whenever `host` is laid out at a clearly different width,
 * so SVG text keeps its size at every viewport. Without ResizeObserver (tests, old engines) the
 * first paint stays.
 */
export function paintAtWidth(
  host: HTMLElement,
  initial: number,
  paint: (width: number) => void,
): void {
  let current = initial;
  paint(current);
  if (typeof ResizeObserver === 'undefined') return;
  new ResizeObserver(() => {
    const width = Math.floor(host.getBoundingClientRect().width);
    if (width >= 120 && Math.abs(width - current) >= 8) {
      current = width;
      paint(width);
    }
  }).observe(host);
}

/** A table for assistive technology and copy-paste, in a visually hidden `tmt-ui-chart-data` box. */
export function dataTable(
  caption: string,
  columns: readonly string[],
  rows: readonly (readonly string[])[],
): HTMLElement {
  const table = htmlEl('table');
  table.append(htmlEl('caption', undefined, caption));
  const head = htmlEl('tr');
  head.append(htmlEl('th', undefined, ''));
  for (const column of columns) {
    const cell = htmlEl('th', undefined, column);
    cell.scope = 'col';
    head.append(cell);
  }
  const thead = htmlEl('thead');
  thead.append(head);
  table.append(thead);
  const body = htmlEl('tbody');
  for (const [label, ...values] of rows) {
    const line = htmlEl('tr');
    const header = htmlEl('th', undefined, label);
    header.scope = 'row';
    line.append(header, ...values.map((value) => htmlEl('td', undefined, value)));
    body.append(line);
  }
  table.append(body);
  // A table ignores overflow, so the clipping box is a wrapper around it.
  const box = htmlEl('div', 'tmt-ui-chart-data');
  box.append(table);
  return box;
}
