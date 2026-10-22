// Checks that the built Colab module carries the app build it shipped beside (#4282): the update
// row compares that stamp with the build the server serves, so a module built before a different
// app build (a reordered or partial build) would show a permanent update row a reload cannot clear.
// Usage: node verify-module-build.mjs <app dist directory>
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ENTRY = /\/assets\/index-[A-Za-z0-9_-]{8}\.js/g;

/** The stamp the module in `directory` carries; throws when it is missing, ambiguous or not the
 * entry the directory's `index.html` loads. */
export function verifyModuleBuild(directory) {
  const module = readFileSync(join(directory, 'assets/colab-module.js'), 'utf8');
  const stamps = [...new Set(module.match(ENTRY) ?? [])];
  if (stamps.length !== 1)
    throw new Error(`colab-module.js must carry exactly one app build, found ${stamps.length}.`);
  const [stamp] = stamps;
  if (!existsSync(join(directory, stamp)))
    throw new Error(`colab-module.js carries ${stamp}, which this build does not contain.`);
  const file = stamp.slice(1).replaceAll(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const loaded = new RegExp(`<script\\b[^>]*\\btype="module"[^>]*\\bsrc="[^"]*${file}"`);
  if (!loaded.test(readFileSync(join(directory, 'index.html'), 'utf8')))
    throw new Error(`colab-module.js carries ${stamp}, but index.html loads another entry.`);
  return stamp;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [directory] = process.argv.slice(2);
  if (!directory) throw new Error('usage: verify-module-build.mjs <dist directory>');
  console.log(`colab module build ok: ${verifyModuleBuild(directory)}`);
}
