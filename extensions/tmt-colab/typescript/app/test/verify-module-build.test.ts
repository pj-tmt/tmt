import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, it } from 'vite-plus/test';
// @ts-expect-error The verifier is a plain script with no type declarations.
import { verifyModuleBuild } from '../../../scripts/verify-module-build.mjs';

// The built Colab module must carry the app build it shipped beside (#4282): a stamp that is
// missing, doubled or not the entry index.html loads would show a permanent update row.
const directories: string[] = [];
afterEach(() => {
  for (const directory of directories.splice(0))
    rmSync(directory, { recursive: true, force: true });
});
function built(options: { module: string; entry?: string; html?: string }): string {
  const directory = mkdtempSync(join(tmpdir(), 'tmt-module-build-'));
  directories.push(directory);
  mkdirSync(join(directory, 'assets'));
  const entry = options.entry ?? 'index-AbC123xY.js';
  writeFileSync(join(directory, 'assets', entry), '');
  writeFileSync(join(directory, 'assets/colab-module.js'), options.module);
  writeFileSync(
    join(directory, 'index.html'),
    options.html ??
      `<html><head><script type="module" crossorigin src="./assets/${entry}"></script></head></html>`,
  );
  return directory;
}

it('accepts a module that carries the entry index.html loads', () => {
  const directory = built({ module: 'const b="/assets/index-AbC123xY.js";export{b}' });
  expect(verifyModuleBuild(directory)).toBe('/assets/index-AbC123xY.js');
  // The same stamp written twice is still one build.
  const twice = built({ module: 'a="/assets/index-AbC123xY.js";b="/assets/index-AbC123xY.js"' });
  expect(verifyModuleBuild(twice)).toBe('/assets/index-AbC123xY.js');
});

it('refuses a module without a stamp', () => {
  expect(() => verifyModuleBuild(built({ module: 'export{}' }))).toThrow(
    /exactly one app build, found 0/,
  );
  // An unhashed or short-hashed entry is not a stamp.
  expect(() =>
    verifyModuleBuild(built({ module: 'a="/assets/index-AbC123x.js";b="/assets/main.js"' })),
  ).toThrow(/found 0/);
});

it('refuses a module that names two different builds', () => {
  const directory = built({
    module: 'a="/assets/index-AbC123xY.js";b="/assets/index-OtherOne.js"',
  });
  expect(() => verifyModuleBuild(directory)).toThrow(/exactly one app build, found 2/);
});

it('refuses a stamp the build does not contain', () => {
  const directory = built({ module: 'a="/assets/index-GoneGone.js"' });
  expect(() => verifyModuleBuild(directory)).toThrow(/does not contain/);
});

it('refuses a stamp older than the entry index.html loads', () => {
  // The app was built again after the module: the module names the previous entry.
  const directory = built({ module: 'a="/assets/index-AbC123xY.js"', entry: 'index-AbC123xY.js' });
  writeFileSync(join(directory, 'assets/index-NewNew12.js'), '');
  writeFileSync(
    join(directory, 'index.html'),
    '<html><head><script type="module" crossorigin src="./assets/index-NewNew12.js"></script></head></html>',
  );
  expect(() => verifyModuleBuild(directory)).toThrow(/index.html loads another entry/);
});

it('requires index.html to load the stamp as a module script, not merely mention it', () => {
  const directory = built({
    module: 'a="/assets/index-AbC123xY.js"',
    html: '<html><head><!-- ./assets/index-AbC123xY.js --><script src="./assets/other.js"></script></head></html>',
  });
  expect(() => verifyModuleBuild(directory)).toThrow(/loads another entry/);
});
