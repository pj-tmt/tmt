// Checks the built page runtime against its reviewed pin (#1693), the way the Remote Firebase build
// is pinned: exact bytes and versions, so a changed dependency or build output is a reviewed diff.
// Usage: node verify-runtime.mjs <app dist directory> [--update]
//   --update  rewrites app/runtime-pin.json from the build, for a deliberate dependency change.
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const MAX_BYTES = 1.5 * 1024 * 1024;
const MAX_THEME_BYTES = 16 * 1024;
const app = new URL('../typescript/app/', import.meta.url).pathname;
const [directory, flag] = process.argv.slice(2);
if (!directory) throw new Error('usage: verify-runtime.mjs <dist directory> [--update]');

const read = (path) => readFileSync(path, 'utf8');
const manifest = JSON.parse(read(join(app, 'package.json')));
const installed = (name) => JSON.parse(read(join(app, 'node_modules', name, 'package.json'))).version;
const packages = Object.fromEntries(
  ['react', 'react-dom', 'htm', '@tailwindcss/browser', 'lucide-react'].map((name) => [name, installed(name)]),
);
const bytes = readFileSync(join(directory, 'assets/colab-runtime.js'));
const themeBytes = readFileSync(join(directory, 'assets/colab-runtime.css'));
const entry = (path, content) => ({
  path,
  sha256: createHash('sha256').update(content).digest('hex'),
  length: content.length,
});
const built = {
  version: 1,
  packages,
  files: [
    entry('assets/colab-runtime.js', bytes),
    entry('assets/colab-runtime.css', themeBytes),
  ],
};

const pinPath = join(app, 'runtime-pin.json');
if (flag === '--update') {
  writeFileSync(pinPath, `${JSON.stringify(built, null, 2)}\n`);
  console.log('runtime-pin.json updated');
  process.exit(0);
}

const failures = [];
const expect = (ok, message) => ok || failures.push(message);
const pin = JSON.parse(read(pinPath));
for (const [name, version] of Object.entries(packages)) {
  const declared = manifest.dependencies[name];
  expect(declared === version, `${name}: package.json says ${declared}, installed ${version}`);
  expect(pin.packages[name] === version, `${name}: pin says ${pin.packages[name]}, installed ${version}`);
}
expect(JSON.stringify(pin.files) === JSON.stringify(built.files), 'runtime bytes differ from runtime-pin.json');
const text = bytes.toString('utf8');
expect(bytes.length <= MAX_BYTES, `runtime is ${bytes.length} bytes, over ${MAX_BYTES}`);
// The frame injects this as an inline script: it must not close its own tag, must not need
// `process`, must not load a source map, and must not need eval (the frame has no unsafe-eval).
expect(!text.toLowerCase().includes('</script'), 'contains a script end tag');
  expect(!text.includes('<' + '!--'), 'contains an HTML comment opener');
expect(!text.includes('process.env'), 'reads process.env');
expect(!text.includes('sourceMappingURL'), 'references a source map');
expect(!/\beval\s*\(/.test(text) && !text.includes('new Function('), 'uses eval or new Function');
// The theme goes into a `<style type="text/tailwindcss">` block: an `@import` there makes the
// browser build skip its own default import of Tailwind, and `</style` would end the block.
const theme = themeBytes.toString('utf8');
expect(themeBytes.length <= MAX_THEME_BYTES, `theme is ${themeBytes.length} bytes, over ${MAX_THEME_BYTES}`);
expect(!theme.includes('@import'), 'theme contains @import');
expect(!theme.toLowerCase().includes('</style'), 'theme contains a style end tag');
expect(!theme.includes('<' + '!--'), 'theme contains an HTML comment opener');
const notices = read(join(directory, 'THIRD-PARTY-NOTICES.txt'));
for (const name of Object.keys(packages))
  expect(notices.includes(`\n## ${name} - ${packages[name]} (`), `notices lack ${name} ${packages[name]}`);

if (failures.length) {
  console.error(`Page runtime check failed:\n- ${failures.join('\n- ')}`);
  process.exit(1);
}
console.log(`page runtime ok: ${bytes.length} bytes, theme ${themeBytes.length} bytes`);
