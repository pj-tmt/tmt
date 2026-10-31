// The component suite owns its frozen runtime prerequisite. Reuse a verified build when present.
import { existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const app = new URL('../typescript/app/', import.meta.url);
const required = [
  'assets/colab-runtime.js',
  'assets/colab-runtime.css',
  'assets/colab-page-ui.js',
  'assets/colab-page-ui.css',
  'THIRD-PARTY-NOTICES.txt',
];
if (required.some((path) => !existsSync(new URL(`dist/${path}`, app)))) {
  console.log('Browser fixture assets missing; building the owned app once.');
  const pnpm = process.env.npm_execpath;
  if (!pnpm) throw new Error('Run this prerequisite through pnpm test:browser.');
  const built = spawnSync(process.execPath, [pnpm, 'run', 'build'], {
    cwd: fileURLToPath(app),
    stdio: 'inherit',
  });
  if (built.error) throw built.error;
  if (built.status !== 0) process.exit(built.status ?? 1);
} else {
  console.log('Reusing existing browser fixture assets.');
}
// A present but invalid build fails the existing pin/style/license gate; it is never silently reused.
const checked = spawnSync(
  process.execPath,
  [
    fileURLToPath(new URL('./verify-runtime.mjs', import.meta.url)),
    fileURLToPath(new URL('dist/', app)),
  ],
  { cwd: fileURLToPath(app), stdio: 'inherit' },
);
if (checked.error) throw checked.error;
process.exit(checked.status ?? 1);
