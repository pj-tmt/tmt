// Appends the licenses of the two page-runtime packages the default build never bundles (htm, and
// the prebuilt Tailwind browser script) to the app's THIRD-PARTY-NOTICES.txt (#1693). React and
// Lucide are bundled by the default build too, so Vite's own notices already carry them.
// Usage: node runtime-notices.mjs <app dist directory>. Running it twice changes nothing.
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const MARK = '<!-- page runtime -->';
const packages = [
  ['htm', 'Apache-2.0'],
  ['@tailwindcss/browser', 'MIT'],
];

const [directory] = process.argv.slice(2);
if (!directory) throw new Error('usage: runtime-notices.mjs <dist directory>');
// Both are direct dependencies of the app, so they sit under its own node_modules.
const modules = new URL('../typescript/app/node_modules/', import.meta.url).pathname;
const file = join(directory, 'THIRD-PARTY-NOTICES.txt');
const current = readFileSync(file, 'utf8');
const base = current.includes(MARK) ? current.slice(0, current.indexOf(MARK)) : current;
let added = `${MARK}\n`;
for (const [name, license] of packages) {
  const root = join(modules, name);
  const { version } = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
  added += `\n## ${name} - ${version} (${license})\n\n${readFileSync(join(root, 'LICENSE'), 'utf8').trim()}\n`;
}
writeFileSync(file, `${base.trimEnd()}\n\n${added}`);
