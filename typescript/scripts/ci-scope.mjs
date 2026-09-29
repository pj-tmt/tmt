import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { runPackedCommand } from './packed-command.mjs';

/**
 * Core surfaces the native Office companion consumes: the storage schema and
 * adapters, the Office adapters and local service, native packaging, the CLI
 * Office facade, and workspace build inputs outside the crates.
 */
const NATIVE_OFFICE_CORE = [
  'rust/crates/tmt-adapters/src/storage/',
  'rust/crates/tmt-adapters/src/office_',
  'rust/crates/tmt-adapters/src/native_install/',
  'rust/crates/tmt-core/src/native_install',
  'rust/crates/tmt-cli/src/office_facade.rs',
];

/**
 * Shared paths the native Office image never reads: prose outside Office,
 * core-only test suites, and E2E scenarios (Office imports only the harness
 * and test support) with their separate image.
 */
const NATIVE_OFFICE_UNRELATED = [
  /^(?!docs\/office\/)(?:docs\/.+|[^/]+)\.md$/,
  /^typescript\/test\/(?:native|tooling)\//,
  /^typescript\/test\/e2e\/(?:[^/]+\.e2e\.test\.ts|Dockerfile)$/,
];

/** Unknown paths fail closed, as they do for the other areas. */
function consumedByNativeOffice(path) {
  if (path.startsWith('rust/crates/')) {
    return NATIVE_OFFICE_CORE.some((prefix) => path.startsWith(prefix));
  }
  if (path.startsWith('rust/')) return true;
  if (path.startsWith('skills/') || path.startsWith('extensions/tmt-squad/')) return false;
  return !NATIVE_OFFICE_UNRELATED.some((pattern) => pattern.test(path));
}

/**
 * Unknown and shared paths run every consumer; deletions are still changes.
 * `nativeOffice` schedules the advisory native Office browser shards only for
 * Office itself or the core surfaces it consumes.
 */
export function selectCiAreas(paths) {
  const selected = { native: false, office: false, nativeOffice: false };
  if (paths.length === 0) return { native: true, office: true, nativeOffice: true };
  for (const path of paths) {
    selected.nativeOffice ||= consumedByNativeOffice(path);
    if (
      path.startsWith('apps/office/') ||
      path.startsWith('extensions/tmt-office/typescript/apps/office/') ||
      path.startsWith('docs/office/')
    ) {
      selected.office = true;
    } else if (
      path.startsWith('rust/') ||
      path.startsWith('skills/') ||
      path.startsWith('extensions/tmt-squad/')
    ) {
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
  process.stdout.write(
    `native=${areas.native}\noffice=${areas.office}\nnative_office=${areas.nativeOffice}\n`
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2));
}
