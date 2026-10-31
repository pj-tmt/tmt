import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { expect, test } from 'vite-plus/test';
import {
  isRenderRectangle,
  MAX_RENDER_SOURCE_BYTES,
  rendererUrl,
  sourceFrameKey,
} from '../src/renderer.js';

const html = readFileSync(new URL('../public/renderer.html', import.meta.url), 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)![1];
test('selection and saved-anchor rectangles share exact bounded cosmetic admission', () => {
  const rect = { x: 12, y: 40, width: 200, height: 24 };
  expect(isRenderRectangle(rect)).toBe(true);
  for (const invalid of [
    null,
    [],
    { ...rect, extra: true },
    { ...rect, width: -1 },
    { ...rect, x: 1_000_001 },
    { ...rect, y: NaN },
    { ...rect, height: Infinity },
    { ...rect, x: '12' },
    { x: 12, y: 40, width: 200 },
  ])
    expect(isRenderRectangle(invalid)).toBe(false);
});
test('paint ordinals reject malformed, duplicate and out-of-count batches', () => {
  const owner = script.slice(script.indexOf('const validQuote ='), script.indexOf('let batch ='));
  const admit = runInNewContext(`${owner}; validHighlightAnchors`, {
    utf8: new TextEncoder(),
  }) as (anchors: unknown, count: unknown) => boolean;
  const first = { id: 'one', selector: { exact: 'A quote', prefix: '', suffix: '' }, ordinal: 1 };
  expect(admit([first, { ...first, id: 'two', ordinal: 3 }], 3)).toBe(true);
  expect(admit([], 0)).toBe(true);
  for (const ordinal of [
    undefined,
    null,
    0,
    -1,
    1.5,
    4,
    NaN,
    Infinity,
    '1',
    Number.MAX_SAFE_INTEGER + 1,
  ])
    expect(admit([{ ...first, ordinal }], 3)).toBe(false);
  for (const count of [-1, 1.5, undefined, null, '3', Infinity])
    expect(admit([first], count)).toBe(false);
  expect(admit([first, { ...first, id: 'two' }], 2)).toBe(false);
  expect(admit([first, { ...first, ordinal: 2 }], 2)).toBe(false);
  expect(admit([{ ...first, label: 'Private comment' }], 1)).toBe(false);
  expect(admit([{ ...first, id: '' }], 1)).toBe(false);
  expect(admit([{ ...first, selector: { ...first.selector, extra: true } }], 1)).toBe(false);
  const { ordinal: _ordinal, ...missing } = first;
  expect(admit([missing], 1)).toBe(false);
});
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

test('hosted short routes load the one root renderer while native mounts keep their renderer', () => {
  for (const route of ['/', '/read/AbCd1234', '/p/AbCd1234', '/colab'])
    expect(rendererUrl(`https://demo-example.web.app${route}`)).toBe(
      'https://demo-example.web.app/renderer.html',
    );
  expect(rendererUrl('https://door.example.test/r/abcd/x/colab/read')).toBe(
    'https://door.example.test/r/abcd/x/colab/renderer.html',
  );
  expect(rendererUrl('http://127.0.0.1:4179/')).toBe('http://127.0.0.1:4179/renderer.html');
});
const runtime = { script: 'window.tmt = Object.freeze({});', style: '' };
const bound = (frame: ReturnType<typeof bootstrap>, extra: Record<string, unknown>) =>
  frame.send({
    ...message(frame.parent),
    data: { ...message(frame.parent).data, ...extra },
  });
test('a bind without a runtime writes the head exactly as before', () => {
  const frame = bootstrap();
  frame.send(message(frame.parent));
  expect(frame.writes).toEqual([
    '<!doctype html><html><head><script>window.__colabRenderStart();delete window.__colabRenderStart;</script></head><body><p>Captured source</p></body></html>',
  ]);
});
test('a bind may carry the bundled runtime, written into the trusted head before the page', () => {
  for (const extra of [{ runtime }, { runtime, layout: 'column' }]) {
    const frame = bootstrap();
    bound(frame, extra);
    expect(frame.writes).toEqual([
      '<!doctype html><html><head><script>window.__colabRenderStart();delete window.__colabRenderStart;</script>' +
        '<style type="text/tailwindcss"></style><script>window.tmt = Object.freeze({});</script></head><body><p>Captured source</p></body></html>',
    ]);
  }
});
test('the runtime is exactly two bounded strings that cannot end their own element', () => {
  const big = (bytes: number) => 'x'.repeat(bytes);
  const refused: unknown[] = [
    null,
    [],
    'script',
    {},
    { script: runtime.script },
    { style: '' },
    { ...runtime, extra: true },
    { script: 1, style: '' },
    { script: runtime.script, style: null },
    { script: big(1.5 * 1024 * 1024 + 1), style: '' },
    { script: runtime.script, style: big(256 * 1024 + 1) },
    { script: 'a</script><script>b', style: '' },
    { script: 'a</SCRIPT>', style: '' },
    { script: `a${'<'}!--b`, style: '' },
    { script: runtime.script, style: 'a</style><b>' },
    { script: runtime.script, style: 'a</STYLE>' },
    { script: runtime.script, style: `a${'<'}!--b` },
  ];
  for (const value of refused) {
    const frame = bootstrap();
    bound(frame, { runtime: value });
    expect(frame.writes, JSON.stringify(value)?.slice(0, 80)).toEqual([]);
  }
  const edge = bootstrap();
  bound(edge, { runtime: { script: big(1.5 * 1024 * 1024), style: big(256 * 1024) } });
  expect(edge.writes).toHaveLength(1);
});
test('the key count allows exactly the optional layout and runtime, nothing else', () => {
  const frame = bootstrap();
  bound(frame, { runtime, extra: true });
  bound(frame, { runtime, layout: 'column', extra: true });
  bound(frame, { layout: 'column', extra: true });
  expect(frame.writes).toEqual([]);
  bound(frame, { runtime: undefined });
  expect(frame.writes).toEqual([]);
});

test('the source part of the frame key: script-free pages share a key, any script keeps its own', () => {
  // A script-free page keeps its key across edits, so an edit updates the open frame (#5022).
  expect(sourceFrameKey('<p>one</p>')).toBe(sourceFrameKey('<p>two</p><style>p{}</style>'));
  // The design-system match is fixed when the frame binds.
  expect(sourceFrameKey('<p>one</p>')).not.toBe(
    sourceFrameKey('<tmt-icon name="check"></tmt-icon>'),
  );
  expect(sourceFrameKey('<p class="tmt-ui-card">a</p>')).toBe(
    sourceFrameKey('<p class="tmt-ui-card">b</p>'),
  );
  // A page that may hold a script, an inline handler or the runtime marker is keyed by its whole
  // source, so any edit of it mounts a fresh frame.
  for (const scripted of [
    '<script>1</script><p>a</p>',
    '<SCRIPT>1</SCRIPT><p>a</p>',
    '<svg><script>1</script></svg>',
    '<button onclick="go()">a</button>',
    '<img src="x"\nonerror = "go()">',
    '<p title="x"onmouseover="go()">a</p>',
    '<meta name="tmt-colab-runtime" content="react"><p>a</p>',
  ]) {
    expect(sourceFrameKey(scripted)).not.toBe(sourceFrameKey('<p>a</p>'));
    expect(sourceFrameKey(scripted)).not.toBe(sourceFrameKey(`${scripted} `));
  }
});

test('closed details hide body targets through every ancestor but preserve the first summary', () => {
  const start = script.indexOf('function hiddenByClosedDetails(');
  const end = script.indexOf('function resolveSlots()', start);
  const hidden = runInNewContext(`${script.slice(start, end)}; hiddenByClosedDetails`) as (
    target: unknown,
  ) => boolean;
  const outer = {
    localName: 'details',
    open: false,
    parentElement: null,
    children: [] as unknown[],
  };
  const inner = {
    localName: 'details',
    open: true,
    parentElement: outer,
    children: [] as unknown[],
  };
  const target = { parentElement: inner };
  const summary = { localName: 'summary', contains: (node: unknown) => node === target };
  expect(hidden(target)).toBe(true);
  outer.open = true;
  expect(hidden(target)).toBe(false);
  inner.open = false;
  expect(hidden(target)).toBe(true);
  inner.children = [summary];
  expect(hidden(target)).toBe(false);
  outer.open = false;
  expect(hidden(target)).toBe(true);
  outer.children = [summary];
  expect(hidden(target)).toBe(false);
  // Only the first summary is exposed by a closed details element.
  inner.children = [{ localName: 'summary', contains: () => false }, summary];
  expect(hidden(target)).toBe(true);
});

test('widget source changes remount React-owned page content while ordinary design text morphs', () => {
  const first = '<tmt-ui-button variant="primary">Save</tmt-ui-button>';
  expect(sourceFrameKey(first)).not.toBe(sourceFrameKey(first.replace('Save', 'Apply')));
  expect(sourceFrameKey(first)).toBe(sourceFrameKey(first));
  expect(sourceFrameKey('<p class="tmt-ui-card">A</p>')).toBe(
    sourceFrameKey('<p class="tmt-ui-card">B</p>'),
  );
});
