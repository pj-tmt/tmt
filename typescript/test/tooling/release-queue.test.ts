import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { describe, expect, it } from 'vitest';

const workflow = readFileSync(
  new URL('../../../.github/workflows/release.yml', import.meta.url),
  'utf8'
);
const step = workflow
  .split('      - name: Run release-please\n')[1]
  .split('\n      # Release pull requests')[0];
const shell = step
  .split('        run: |\n')[1]
  .split('\n')
  .map((line) => line.slice(10))
  .join('\n');
// Run 36971196291 / job 110725334139, 2026-10-02. Timestamps and stack paths
// removed; the actual pinned CLI error/cause/status/request framing is retained.
const queued = readFileSync(
  new URL('../fixtures/release-please-queued-ref.txt', import.meta.url),
  'utf8'
);

function execute(log: string, failCommand = 'release-pr', live = 'true', teeFails = false) {
  const directory = mkdtempSync(path.join(tmpdir(), 'tmt-release-queue-'));
  try {
    writeFileSync(path.join(directory, 'failure.txt'), log);
    writeFileSync(
      path.join(directory, 'pnpm'),
      `#!/bin/sh
printf '%s\\n' "$3" >> "$RUNNER_TEMP/commands"
if [ "$3" = "$FAIL_COMMAND" ]; then
  cat "$RUNNER_TEMP/failure.txt"
  exit 19
fi
echo 'release-please succeeded'
`,
      { mode: 0o700 }
    );
    if (teeFails)
      writeFileSync(path.join(directory, 'tee'), '#!/bin/sh\ncat >/dev/null\nexit 23\n', {
        mode: 0o700,
      });
    const summary = path.join(directory, 'summary');
    const result = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', shell], {
      cwd: directory,
      env: {
        PATH: `${directory}:${process.env.PATH}`,
        RUNNER_TEMP: directory,
        GITHUB_STEP_SUMMARY: summary,
        GITHUB_REPOSITORY: 'pj-tmt/tmt',
        LIVE: live,
        RELEASE_TOKEN: 'fixture',
        FAIL_COMMAND: failCommand,
      },
      encoding: 'utf8',
      timeout: 5000,
    });
    if (result.error) throw result.error;
    return {
      status: result.status,
      output: result.stdout + result.stderr,
      summary: readFileSync(summary, 'utf8'),
      commands: readFileSync(path.join(directory, 'commands'), 'utf8'),
    };
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

describe('queued release PR branch update', () => {
  it('logs the exact ref-update 422 as a no-op and still runs github-release', () => {
    const result = execute(queued);
    expect(result.status).toBe(0);
    expect(result.summary).toContain('Release PR is queued; leaving its branch unchanged.');
    expect(result.summary).toContain('first main push after it merges');
    expect(result.commands).toBe('release-pr\ngithub-release\n');
  });

  it('preserves an ordinary successful run without a queue notice', () => {
    const result = execute('', 'none');
    expect(result.status).toBe(0);
    expect(result.commands).toBe('release-pr\ngithub-release\n');
    expect(result.summary).not.toContain('Release PR is queued');
  });

  it.each([
    [
      'different 422',
      queued.replace(
        'are queued for merging cannot be updated.',
        'are protected and cannot be updated.'
      ),
    ],
    ['5xx', queued.replaceAll('status: 422', 'status: 503')],
    ['other response status', queued.replace('status: 422', 'status: 500')],
    [
      'other ref',
      queued.replaceAll('release-please--branches--main--components--tmt-cli', 'feature-branch'),
    ],
    ['other repository', queued.replaceAll('repos/pj-tmt/tmt/', 'repos/pj-tmt/other/')],
    ['other method', queued.replace("method: 'PATCH'", "method: 'POST'")],
    ['missing status', queued.replaceAll('status: 422,', '')],
    ['missing request context', queued.replace('    request: {', '    unknown: {')],
    ['message only', 'queued for merging cannot be updated'],
    ['another error', `${queued}\nError: second update failed\n`],
    ['changed response text', queued.replace('associated pull request.', 'associated request.')],
  ])('fails closed for %s', (_name, log) => {
    const result = execute(log);
    expect(result.status).toBe(19);
    expect(result.commands).toBe('release-pr\n');
    expect(result.summary).not.toContain('Release PR is queued');
  });

  it('does not suppress even the same error from github-release', () => {
    const result = execute(queued, 'github-release');
    expect(result.status).toBe(19);
    expect(result.summary).not.toContain('Release PR is queued');
  });

  it('preserves a failed summary write instead of masking it with the queue no-op', () => {
    const result = execute(queued, 'release-pr', 'true', true);
    expect(result.status).toBe(23);
    expect(result.summary).not.toContain('Release PR is queued');
  });
});
