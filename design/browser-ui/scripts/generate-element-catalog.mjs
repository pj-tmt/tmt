// Build-time evidence only. No dev gallery or test runner is imported by a product.
import { readFile, writeFile } from 'node:fs/promises';
import { existsSync, globSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { browserElements } from '../src/elements-manifest.ts';
import { covered } from '../gallery/covered.ts';

const root = new URL('../', import.meta.url);
const require = createRequire(new URL('../package.json', import.meta.url));
const mode = process.argv[2];
if (!['--write', '--check'].includes(mode)) throw new Error('Use --write or --check');
const listed = spawnSync(
  process.execPath,
  [
    require.resolve('@playwright/test/cli'),
    'test',
    '--list',
    '--reporter=json',
    'e2e/elements.spec.ts',
  ],
  { cwd: fileURLToPath(root), encoding: 'utf8', timeout: 60_000, maxBuffer: 4 * 1024 * 1024 },
);
if (listed.status !== 0) throw new Error(`Element test inventory failed: ${listed.stderr}`);
const titles = [];
const collect = (suite) => {
  for (const spec of suite.specs ?? []) titles.push(spec.title);
  for (const child of suite.suites ?? []) collect(child);
};
for (const suite of JSON.parse(listed.stdout).suites) collect(suite);
const read = (name) => readFile(new URL(name, root), 'utf8');
const readme = await read('README.md');
const anchors = new Set(
  [...readme.matchAll(/^#{1,6} (.+)$/gm)].map((match) =>
    match[1]
      .toLowerCase()
      .replace(/[^a-z0-9 -]/g, '')
      .replaceAll(' ', '-'),
  ),
);
const tests = await Promise.all(
  [
    ...globSync('test/**/*.test.{ts,tsx}', { cwd: fileURLToPath(root) }),
    ...globSync('e2e/**/*.spec.ts', { cwd: fileURLToPath(root) }),
  ].map(async (file) => ({ file, text: await read(file) })),
);
const entries = browserElements.map((entry) => {
  const expected = entry.examples.flatMap((example) =>
    [1440, 390].flatMap((width) =>
      ['light', 'dark'].map(
        (theme) => `element ${entry.tag} / ${example.state} / ${width} ${theme}`,
      ),
    ),
  );
  const test = expected.length > 0 && expected.every((title) => titles.includes(title));
  const tokens = [];
  return {
    tag: entry.tag,
    docs: anchors.has(entry.docs.split('#')[1]),
    gallery:
      covered[entry.component] === entry.gallery &&
      existsSync(new URL(`src/${entry.source}`, root)),
    test,
    tests: expected,
    tokens,
  };
});
for (const entry of entries) {
  const definition = browserElements.find((item) => item.tag === entry.tag);
  const css = new URL(`src/${definition.source.replace(/\.tsx?$/, '.css')}`, root);
  entry.tokens = existsSync(css)
    ? [...new Set((await readFile(css, 'utf8')).match(/--tmt-ui-[a-z0-9-]+/g) ?? [])].sort()
    : [...definition.tokens];
}
const source = await read('src/react.ts');
const sources = new Map();
for (const match of source.matchAll(/export\s*\{([^}]+)\}\s*from\s*['"]([^'"]+)['"]/g)) {
  for (const name of match[1].split(',').map((name) => name.trim())) {
    if (/^(?:Browser|Brand|Busy)[A-Z]/.test(name)) sources.set(name, match[2]);
  }
}
const inventory = [...sources].map(([component, source]) => ({
  component,
  source,
  docs: readme.includes(component),
  gallery: component in covered,
  test: tests.some(
    (entry) => entry.text.includes(component) && /\b(?:it|test)(?:\.\w+)?\s*\(/.test(entry.text),
  ),
  tag: browserElements.find((entry) => entry.component === component)?.tag ?? null,
}));
const output = `${JSON.stringify({ version: 1, entries, inventory }, null, 2)}\n`;
const file = new URL('generated/elements-catalog.json', root);
if (mode === '--write') await writeFile(file, output);
else if ((await readFile(file, 'utf8').catch(() => '')) !== output)
  throw new Error('Element catalog evidence is stale');
console.log(
  `Element catalog: ${entries.length} widgets, ${titles.length} collected parity tests, ${inventory.length} presentation exports`,
);
