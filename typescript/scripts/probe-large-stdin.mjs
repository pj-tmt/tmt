// Temporary read-only #4443 incident reproduction; never dispatches or publishes a release.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { cpus, release } from 'node:os';
import path from 'node:path';

const directory = path.resolve('stdin-probe-results');
mkdirSync(directory, { recursive: true });
const environment = { ...process.env, GITHUB_OUTPUT: '', GITHUB_STEP_SUMMARY: '' };
const capture = spawnSync(
  'gh',
  ['api', '--paginate', '--slurp', `repos/${process.env.GITHUB_REPOSITORY}/releases`],
  {
    encoding: 'utf8',
    timeout: 90_000,
    maxBuffer: 128 * 1024 * 1024,
    env: environment,
  }
);
assert.equal(capture.status, 0, capture.stderr);
const pages = JSON.parse(capture.stdout);
assert(Array.isArray(pages) && pages.every(Array.isArray));
const draft = pages.flat().find((row) => row.draft === true && /^v[0-9]/.test(row.tag_name));
assert(draft, 'No existing CLI draft is available for the read-only reproduction');
const catalogue = path.join(directory, 'catalogue.json');
writeFileSync(catalogue, capture.stdout);
const results = [];
for (const [name, producer] of [
  ['incident-command', 'gh api --paginate --slurp "repos/$GITHUB_REPOSITORY/releases"'],
  ['captured-catalogue', 'cat "$TMT_DIAGNOSTIC_CATALOGUE"'],
]) {
  const statusFile = path.join(directory, `${name}.statuses`);
  const command = `${producer} | node typescript/scripts/plan-release-builds.mjs --product cli --tag "$TMT_DIAGNOSTIC_TAG"
statuses=("\${PIPESTATUS[@]}")
printf '%s\\n' "\${statuses[@]}" > "$TMT_DIAGNOSTIC_STATUSES"`;
  const child = spawnSync('/bin/bash', ['-c', command], {
    encoding: 'utf8',
    timeout: 120_000,
    maxBuffer: 1024 * 1024,
    env: {
      ...environment,
      TMT_DIAGNOSTIC_TAG: draft.tag_name,
      TMT_DIAGNOSTIC_CATALOGUE: catalogue,
      TMT_DIAGNOSTIC_STATUSES: statusFile,
    },
  });
  writeFileSync(path.join(directory, `${name}.stdout`), child.stdout ?? '');
  writeFileSync(path.join(directory, `${name}.stderr`), child.stderr ?? '');
  const statuses = child.error
    ? []
    : readFileSync(statusFile, 'utf8').trim().split('\n').map(Number);
  let exactSelection = false;
  if (statuses.length === 2 && statuses.every((status) => status === 0)) {
    const matrix = JSON.parse(
      child.stdout
        .split('\n')
        .find((line) => line.startsWith('matrix='))
        .slice(7)
    );
    exactSelection =
      matrix.include.length === 1 &&
      matrix.include[0].tag === draft.tag_name &&
      matrix.include[0].sha === draft.target_commitish &&
      child.stdout.split('\n').includes('any=true');
  }
  results.push({
    name,
    statuses,
    shellStatus: child.status,
    shellSignal: child.signal,
    error: child.error?.message ?? '',
    exactSelection,
  });
}
const report = {
  node: process.version,
  platform: process.platform,
  arch: process.arch,
  cpu: cpus()[0]?.model,
  kernel: release(),
  image: process.env.ImageOS,
  imageVersion: process.env.ImageVersion,
  tag: draft.tag_name,
  sha: draft.target_commitish,
  catalogue: {
    bytes: Buffer.byteLength(capture.stdout),
    pages: pages.length,
    records: pages.flat().length,
    sha256: createHash('sha256').update(capture.stdout).digest('hex'),
  },
  plannerSha256: createHash('sha256')
    .update(readFileSync('typescript/scripts/plan-release-builds.mjs'))
    .digest('hex'),
  results,
};
writeFileSync(path.join(directory, 'report.json'), JSON.stringify(report, null, 2) + '\n');
console.log(JSON.stringify(report, null, 2));
// Both successful reproduction and a crash are evidence. Unexpected acquisition/selection errors fail.
if (
  results.some(
    (row) => row.error || (row.statuses.every((status) => status === 0) && !row.exactSelection)
  )
)
  process.exitCode = 1;
