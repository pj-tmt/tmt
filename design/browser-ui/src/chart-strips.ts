import { htmlEl } from './chart-dom.js';
import { type StripsOptions, stripsModel } from './chart-strips-model.js';

export type { StripRow, StripsOptions } from './chart-strips-model.js';

/** Rows of fixed slots: when each seat was active, scaled to that row's busiest slot. */
export function createStrips(options: StripsOptions): HTMLElement {
  const figure = htmlEl('figure', 'tmt-ui-chart');
  figure.append(htmlEl('figcaption', 'tmt-ui-chart-title', options.title));
  if (!options.rows.length) {
    figure.append(htmlEl('p', 'tmt-ui-chart-empty', options.empty ?? 'No agents observed.'));
    return figure;
  }
  const list = htmlEl('ul', 'tmt-ui-chart-strips');
  for (const row of stripsModel(options).rows) {
    const item = htmlEl('li', 'tmt-ui-chart-strip-row');
    item.setAttribute('aria-label', row.summary);
    const name = htmlEl('span', 'tmt-ui-chart-strip-label', row.label);
    name.title = row.label;
    const strip = htmlEl('div', `tmt-ui-chart-strip tmt-ui-chart-strip-${options.slotCount}`);
    for (const slot of row.slots) {
      const node = htmlEl('span', 'tmt-ui-chart-slot');
      if (slot.level !== null) node.classList.add(`tmt-ui-chart-slot-${slot.level}`);
      node.title = slot.title;
      strip.append(node);
    }
    item.append(name, strip);
    list.append(item);
  }
  const ends = htmlEl('div', 'tmt-ui-chart-strip-ends');
  ends.setAttribute('aria-hidden', 'true');
  ends.append(
    htmlEl('span', undefined, options.ends[0]),
    htmlEl('span', undefined, options.ends[1]),
  );
  figure.append(list, ends);
  return figure;
}
