import { globSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { runPackedCommand } from './packed-command.mjs';

const toolingRoot = fileURLToPath(new URL('..', import.meta.url));
const codeTargets = [
  'test',
  'scripts/*.mjs',
  'tsconfig.json',
  'tsconfig.e2e.json',
  'vitest.config.ts',
  'package.json',
  '../.github/workflows/ci.yml',
  '../.github/workflows/office-browser.yml',
  '../.github/components.json',
  '../.github/release-parity.json',
  '../extensions/tmt-office/typescript/services/office/firebase.json',
  '../extensions/tmt-office/typescript/services/office/compose.yaml',
  '../extensions/tmt-office/contracts/*.json',
];
const docsTargets = [
  '../AGENTS.md',
  '../ARCHITECTURE.md',
  '../CONVENTIONS.md',
  '../DEVELOPMENT.md',
  '../rust/archive/NATIVE-INSTALL.md',
  '../contracts/extension-api.md',
  '../contracts/request-response-v1.md',
  '../.agents/skills/**/*.md',
  '../.github/pull_request_template.md',
  '../extensions/tmt-office/docs/**/*.md',
  '../extensions/tmt-office/typescript/services/office/README.md',
  '../extensions/tmt-office/contracts/*.md',
  '../extensions/tmt-colab/contracts/*.md',
];

/** Oxfmt 0.70 rejects parent-relative paths and does not expand absolute globs. */
export function expandFormatterTargets(patterns, cwd) {
  return [
    ...new Set(
      patterns.flatMap((pattern) => {
        const matches = globSync(pattern, { cwd });
        if (matches.length === 0) throw new Error(`Formatter target matched no files: ${pattern}`);
        return matches.map((match) => path.resolve(cwd, match));
      })
    ),
  ].sort();
}

export function formatterTargets(docs = false) {
  return expandFormatterTargets(docs ? docsTargets : codeTargets, toolingRoot);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const options = process.argv.slice(2);
  if (options.some((option) => !['--docs', '--check'].includes(option))) {
    throw new Error('Usage: format-workspace.mjs [--docs] [--check]');
  }
  const docs = options.includes('--docs');
  const args = [
    'fmt',
    '--config',
    'vitest.config.ts',
    options.includes('--check') ? '--check' : '--write',
  ];
  // Code keeps .prettierignore's Markdown exclusion; docs override it exactly as before.
  if (docs) args.push('--ignore-path', '../.gitignore');
  args.push(...formatterTargets(docs));
  const vp = fileURLToPath(new URL('../bin/vp', import.meta.resolve('vite-plus')));
  process.stdout.write(
    runPackedCommand(process.execPath, [vp, ...args], {
      cwd: toolingRoot,
      env: process.env,
      timeoutMs: 60_000,
    })
  );
}
