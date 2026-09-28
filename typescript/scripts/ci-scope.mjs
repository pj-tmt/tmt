import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { runPackedCommand } from './packed-command.mjs';

/** Unknown and shared paths run every consumer; deletions are still changes. */
export function selectCiAreas(paths) {
  const selected = { native: false, office: false };
  if (paths.length === 0) return { native: true, office: true };
  for (const path of paths) {
    if (
      path.startsWith('apps/office/') ||
      path.startsWith('extensions/tmt-office/typescript/apps/office/') ||
      path.startsWith('docs/office/')
    ) {
      selected.office = true;
    } else if (path.startsWith('rust/') || path.startsWith('skills/')) {
      selected.native = true;
    } else {
      // Includes contracts, security, lockfiles, scripts, tests and CI itself.
      selected.native = true;
      selected.office = true;
    }
  }
  return selected;
}

export function ciGatePasses(selected, results) {
  if (!['true', 'false'].includes(selected) || results.length === 0) return false;
  const expected = selected === 'true' ? 'success' : 'skipped';
  return results.every((result) => result === expected);
}

export function readChangedCiAreas(base, head, cwd) {
  if ([base, head].some((sha) => !/^[a-f0-9]{40}$/.test(sha ?? ''))) {
    throw new Error('Expected exact base and head commit SHAs.');
  }
  const changed = runPackedCommand(
    'git',
    ['diff', '--no-renames', '--name-only', '-z', `${base}...${head}`, '--'],
    { cwd, env: process.env }
  );
  return selectCiAreas(changed.split('\0').filter(Boolean));
}

function main(args) {
  if (args[0] === 'gate') {
    if (!ciGatePasses(args[1], args.slice(2))) {
      throw new Error(
        'Selected CI work did not complete successfully, or skip evidence is invalid.'
      );
    }
    return;
  }
  if (args.length !== 2) {
    throw new Error('Expected exact base and head commit SHAs.');
  }
  const areas = readChangedCiAreas(
    args[0],
    args[1],
    fileURLToPath(new URL('../../', import.meta.url))
  );
  process.stdout.write(`native=${areas.native}\noffice=${areas.office}\n`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2));
}
