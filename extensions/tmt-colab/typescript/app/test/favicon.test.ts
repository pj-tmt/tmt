import { readFileSync } from 'node:fs';
import { expect, test } from 'vite-plus/test';

const canonical = readFileSync(
  new URL('../../../../../design/browser-ui/src/favicon.svg', import.meta.url),
);
for (const path of ['../index.html', '../public/reader.html']) {
  test(`${path} carries the exact shared favicon bytes`, () => {
    const html = readFileSync(new URL(path, import.meta.url), 'utf8');
    const links = html.match(/<link\b[^>]*\brel="icon"[^>]*>/g);
    expect(links).toHaveLength(1);
    const href = links![0].match(/\bhref="([^"]+)"/)![1];
    expect(href.startsWith('data:image/svg+xml;base64,')).toBe(true);
    expect(Buffer.from(href.slice('data:image/svg+xml;base64,'.length), 'base64')).toEqual(
      canonical,
    );
  });
}
test('the sandboxed renderer has no tab icon', () => {
  expect(readFileSync(new URL('../public/renderer.html', import.meta.url), 'utf8')).not.toMatch(
    /rel="icon"/,
  );
});
