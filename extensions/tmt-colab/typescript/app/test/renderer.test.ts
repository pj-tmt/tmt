import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { expect, test } from 'vite-plus/test';
import { MAX_RENDER_SOURCE_BYTES } from '../src/renderer.js';

const html = readFileSync(new URL('../public/renderer.html', import.meta.url), 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)![1];
function bootstrap() {
  const parent = {};
  let listener: ((event: unknown) => void) | undefined;
  const writes: string[] = [];
  const window = {
    parent,
    addEventListener: (_: string, value: typeof listener) => {
      listener = value;
    },
    removeEventListener: (_: string, value: typeof listener) => {
      if (listener === value) listener = undefined;
    },
  };
  runInNewContext(script, {
    window,
    TextEncoder,
    document: {
      open() {},
      close() {},
      write(value: string) {
        writes.push(value);
      },
    },
  });
  return { parent, writes, send: (event: unknown) => listener?.(event) };
}
function message(parent: object, source = '<p>Captured source</p>') {
  const sourceDigest = 'a'.repeat(64);
  return {
    source: parent,
    origin: 'null',
    data: {
      type: 'colab.render.bind',
      renderId: `00000000-0000-4000-8000-000000000001:${sourceDigest}`,
      sourceDigest,
      source,
      theme: 'light',
    },
    ports: [{}],
  };
}
test('only a valid parent message initializes source once, independent of origin', () => {
  const frame = bootstrap();
  frame.send(message({}));
  frame.send({ ...message(frame.parent), data: null });
  frame.send({ ...message(frame.parent), ports: [] });
  frame.send({ ...message(frame.parent), data: { ...message(frame.parent).data, extra: true } });
  for (const theme of [undefined, null, 'auto', 'DARK', ['dark'], {}])
    frame.send({ ...message(frame.parent), data: { ...message(frame.parent).data, theme } });
  const { theme: _theme, ...missingTheme } = message(frame.parent).data;
  frame.send({ ...message(frame.parent), data: missingTheme });
  frame.send({
    ...message(frame.parent),
    data: { ...message(frame.parent).data, renderId: 'stale' },
  });
  expect(frame.writes).toEqual([]);
  frame.send({ ...message(frame.parent), origin: 'https://parent.example' });
  expect(frame.writes).toHaveLength(1);
  expect(frame.writes[0]).toContain('<p>Captured source</p>');
  frame.send(message(frame.parent, '<p>Later source</p>'));
  expect(frame.writes).toHaveLength(1);
});
test('both exact theme values initialize source', () => {
  for (const theme of ['light', 'dark']) {
    const frame = bootstrap();
    frame.send({ ...message(frame.parent), data: { ...message(frame.parent).data, theme } });
    expect(frame.writes).toHaveLength(1);
  }
});
test('renderer admission bounds UTF-8 bytes rather than character count', () => {
  const frame = bootstrap();
  frame.send(message(frame.parent, 'é'.repeat(MAX_RENDER_SOURCE_BYTES / 2 + 1)));
  expect(frame.writes).toEqual([]);
  frame.send(message(frame.parent, 'x'.repeat(MAX_RENDER_SOURCE_BYTES)));
  expect(frame.writes).toHaveLength(1);
});
