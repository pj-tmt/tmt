import { createElement } from 'react';
import { expect, it } from 'vite-plus/test';
import { renderToStaticMarkup } from 'react-dom/server';
import { WorkBand } from '../src/work-band.js';

it('names empty progress while retaining the empty meter', () => {
  const html = renderToStaticMarkup(
    createElement(WorkBand, {
      work: { version: 1, next: 1, order: [], tasks: {} },
      progress: { done: 0, inProgress: 0, todo: 0, unknown: 0, total: 0 },
    }),
  );
  expect(html).toContain('No sub-pages or tasks');
  expect(html).toContain('work-segments');
});

it('shows each known sub-page creator after its state', () => {
  const html = renderToStaticMarkup(
    createElement(WorkBand, {
      work: { version: 1, next: 1, order: [], tasks: {} },
      subpages: [{ id: 'child', title: 'Status panel', state: 'in-progress', creator: 'feat-1' }],
    }),
  );
  expect(html).toContain('In progress · feat-1');
});
