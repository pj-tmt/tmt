import { runPackedCommand } from './packed-command.mjs';

export function isRepository(value) {
  return (
    typeof value === 'string' &&
    value.trim() === value &&
    /^[A-Za-z0-9_-][A-Za-z0-9_.-]*\/[A-Za-z0-9_-][A-Za-z0-9_.-]*$/.test(value)
  );
}

function checked(value) {
  if (!isRepository(value)) throw new Error('REPOSITORY: expected OWNER/REPO.');
  return value;
}

/** Resolve once at the CLI boundary; collectors receive the selected repository explicitly. */
export function resolveRepository({
  repo,
  env = process.env,
  cwd = process.cwd(),
  execute = runPackedCommand,
} = {}) {
  if (repo !== undefined) return checked(repo);
  if (env.GITHUB_REPOSITORY !== undefined) return checked(env.GITHUB_REPOSITORY);
  let resolved;
  try {
    // gh follows repository renames, unlike a stale origin URL.
    resolved = execute(
      'gh',
      ['repo', 'view', '--json', 'nameWithOwner', '--jq', '.nameWithOwner'],
      { env, cwd, timeoutMs: 10_000, maxBuffer: 4096 }
    ).trim();
  } catch {
    let remote;
    try {
      remote = execute('git', ['remote', 'get-url', 'origin'], {
        env,
        cwd,
        timeoutMs: 3000,
        maxBuffer: 4096,
      }).trim();
    } catch {
      throw new Error(
        'REPOSITORY: cannot discover GitHub repository; set --repo or GITHUB_REPOSITORY.'
      );
    }
    const match =
      /^(?:https:\/\/github\.com\/|git@github\.com:|ssh:\/\/git@github\.com\/)([^/?#]+\/[^/?#]+?)(?:\.git)?\/?$/.exec(
        remote
      );
    if (!match) throw new Error('REPOSITORY: origin must be a GitHub repository; set --repo.');
    resolved = match[1];
  }
  return checked(resolved);
}
