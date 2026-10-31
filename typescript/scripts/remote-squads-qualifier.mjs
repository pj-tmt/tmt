// Runs Remote's two-machine squads emulator qualifier for the advisory workflow (#5767). The
// Remote squads README owns the command; this reads its two lines at run time so the job
// cannot drift from it. Remote's test_emulator_guard forbids the default-off feature anywhere
// under `.github/`, so the workflow calls this script instead of spelling the command.
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const QUALIFIER_README = 'extensions/tmt-remote/rust/tmt-remote/src/squads/README.md';
const root = fileURLToPath(new URL('../../', import.meta.url));

/** The README's build and emulator lines; any other shape is refused rather than guessed. */
export function qualifierCommands(readme) {
  const block = readme
    .split('## Native CLI emulator qualifier\n')[1]
    ?.split('```sh\n')[1]
    ?.split('```')[0];
  const lines = block?.trim().split('\n') ?? [];
  const inner = lines[1]?.match(/ '(cargo test [^']+)'$/)?.[1];
  const [selection, harness] = inner?.split(' -- ') ?? [];
  if (
    lines.length !== 2 ||
    !lines[0].includes(' cargo build ') ||
    !lines[1].includes(' firebase emulators:exec ') ||
    !harness?.includes('--ignored') ||
    !/ \S+::\S+$/.test(selection ?? '')
  )
    throw new Error(
      `${QUALIFIER_README} must hold exactly the build and emulator qualifier lines.`
    );
  // The same test target without the filter or harness flags compiles the qualifier ahead of
  // the emulators, so the qualifier step's timeout bounds only the run.
  const environment = lines[0].match(/^(?:[A-Z_]+=\S+ )*/)[0];
  const compile = `${environment}${selection.replace(/ \S+$/, '')} --no-run`;
  return { build: [lines[0], compile], qualify: [lines[1]] };
}

function main([phase]) {
  const commands = qualifierCommands(readFileSync(resolve(root, QUALIFIER_README), 'utf8'));
  if (phase !== 'build' && phase !== 'qualify')
    throw new Error('usage: remote-squads-qualifier.mjs build|qualify');
  for (const command of commands[phase]) {
    process.stdout.write(`$ ${command}\n`);
    const result = spawnSync('sh', ['-c', command], { cwd: root, stdio: 'inherit' });
    if (result.error) throw result.error;
    if (result.status !== 0) process.exit(result.status ?? 1);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2));
}
