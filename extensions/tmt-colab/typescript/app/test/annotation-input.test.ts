import { expect, it } from 'vite-plus/test';
import { validateProjection } from '../src/fold-protocol.js';

it('accepts bounded publisher metadata and rejects type, control and Unicode byte overflows', () => {
  for (const publisherAgent of ['publisher', 'é'.repeat(64)])
    expect(() => validateProjection({ source: '', title: '', publisherAgent })).not.toThrow();
  for (const publisherAgent of [null, 1, '', 'é'.repeat(65), 'line\nbreak'])
    expect(() => validateProjection({ source: '', title: '', publisherAgent })).toThrow();
});
