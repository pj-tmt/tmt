import { expect, it } from 'vitest';
import { localTransport } from '../src/transport.js';
import { captureRender, MAX_RENDER_SOURCE_BYTES } from '../src/renderer.js';
import { createHash } from 'node:crypto';

it('keeps local snapshots detached and rejects unknown pages', async () => {
  const source = { id: 'one', title: 'One', sharing: 'private' as const, source: '<h1>One</h1>' };
  const transport = localTransport('Local', [source]);
  source.source = 'changed outside adapter';
  const page = await transport.page('one');
  expect(page.source).toBe('<h1>One</h1>');
  await expect(transport.page('missing')).rejects.toThrow('Unknown page');
  const home = await transport.spaceHome();
  expect(home.pages).toEqual([{ id: 'one', title: 'One', sharing: 'private' }]);
  expect(home.pages[0]).not.toHaveProperty('source');
});
it('binds fresh render IDs to the exact UTF-8 bytes, not a state vector', async () => {
  const source = '<p>😀 &amp; café</p>';
  const a = await captureRender(source),
    b = await captureRender(source);
  expect(a.sourceDigest).toBe(createHash('sha256').update(source).digest('hex'));
  expect(a.renderId.endsWith(':' + a.sourceDigest)).toBe(true);
  expect(a.renderId).not.toBe(b.renderId);
  expect((await captureRender(source + ' ')).sourceDigest).not.toBe(a.sourceDigest);
});
it('accepts the contract 2 MiB boundary and rejects excess UTF-8 bytes', async () => {
  expect(MAX_RENDER_SOURCE_BYTES).toBe(2 * 1024 * 1024);
  await expect(captureRender('x'.repeat(MAX_RENDER_SOURCE_BYTES))).resolves.toHaveProperty(
    'sourceDigest',
  );
  await expect(captureRender('é'.repeat(MAX_RENDER_SOURCE_BYTES / 2) + 'x')).rejects.toThrow(
    'Page exceeds preview limit',
  );
});
