// Temporary read-only #4443 qualification; removed after the hosted proof.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { cpus, release } from 'node:os';
import path from 'node:path';

const directory = path.resolve('stdin-reader-proof');
mkdirSync(directory, { recursive: true });
const environment = { ...process.env, GITHUB_OUTPUT: '', GITHUB_STEP_SUMMARY: '' };
const baselinePath = 'typescript/scripts/plan-release-builds-baseline.mjs';
const bufferPath = 'typescript/scripts/plan-release-builds-buffer-control.mjs';
const fixedPath = 'typescript/scripts/plan-release-builds.mjs';
const old = spawnSync('git', ['show', `${process.env.TMT_BASE_SHA}:${fixedPath}`], {
  encoding: 'utf8',
  timeout: 10_000,
});
assert.equal(old.status, 0, old.stderr);
assert(old.stdout.includes("readFileSync(0, 'utf8')"), 'Baseline must retain the incident reader');
writeFileSync(baselinePath, old.stdout);
writeFileSync(
  bufferPath,
  old.stdout.replace("readFileSync(0, 'utf8')", "readFileSync(0).toString('utf8')")
);
const live = 'gh api --paginate --slurp "repos/$GITHUB_REPOSITORY/releases"';
const acquisition = spawnSync(
  'gh',
  ['api', '--paginate', '--slurp', `repos/${process.env.GITHUB_REPOSITORY}/releases`],
  { env: environment, encoding: 'utf8', timeout: 90_000, maxBuffer: 128 * 1024 * 1024 }
);
assert.equal(acquisition.status, 0, acquisition.stderr);
const pages = JSON.parse(acquisition.stdout);
assert(Array.isArray(pages) && pages.every(Array.isArray));
const draft = pages.flat().find((row) => row.draft === true && /^v[0-9]/.test(row.tag_name));
assert(draft, 'An existing CLI draft is required; never create one for this proof');
writeFileSync(path.join(directory, 'selection-catalogue.json'), acquisition.stdout);
const results = [];
const describe = (file) => {
  const bytes = readFileSync(file);
  return { bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') };
};
try {
  for (const [name, source, measured] of [
    ['baseline-exact', baselinePath, false],
    ['baseline-measured', baselinePath, true],
    ['fixed-measured', fixedPath, true],
    ['fixed-exact', fixedPath, false],
    ['baseline-file-control', baselinePath, false],
    ['buffer-overload-control', bufferPath, false],
  ]) {
    const capture = path.join(directory, `${name}.catalogue.json`);
    const producer = name === 'baseline-file-control' ? 'cat "$TMT_CONTROL"' : live;
    // The extra tee is explicitly separate from the untouched incident command.
    // warn-nopipe drains the real producer into the file even if the old reader crashes.
    const command = `${producer}${measured ? ' | tee --output-error=warn-nopipe "$TMT_CAPTURE"' : ''} | node "$TMT_PLANNER" --product cli --tag "$TMT_TAG"
statuses=("\${PIPESTATUS[@]}")
printf '%s\\n' "\${statuses[@]}" > "$TMT_STATUSES"`;
    const statusesPath = path.join(directory, `${name}.statuses`);
    const child = spawnSync('/bin/bash', ['-c', command], {
      encoding: 'utf8',
      timeout: 120_000,
      maxBuffer: 1024 * 1024,
      env: {
        ...environment,
        TMT_CAPTURE: capture,
        TMT_CONTROL: path.join(directory, 'baseline-measured.catalogue.json'),
        TMT_PLANNER: source,
        TMT_TAG: draft.tag_name,
        TMT_STATUSES: statusesPath,
      },
    });
    writeFileSync(path.join(directory, `${name}.stdout`), child.stdout ?? '');
    writeFileSync(path.join(directory, `${name}.stderr`), child.stderr ?? '');
    const statuses = child.error
      ? []
      : readFileSync(statusesPath, 'utf8').trim().split('\n').map(Number);
    const selected = child.stdout?.split('\n').find((line) => line.startsWith('matrix='));
    const exactSelection = Boolean(
      selected &&
      JSON.stringify(JSON.parse(selected.slice(7))) ===
        JSON.stringify({ include: [{ tag: draft.tag_name, sha: draft.target_commitish }] }) &&
      child.stdout.split('\n').includes('any=true')
    );
    const row = {
      name,
      statuses,
      error: child.error?.message ?? '',
      exactSelection,
      ...(measured ? { catalogue: describe(capture) } : {}),
    };
    results.push(row);
    if (name.startsWith('fixed') && statuses.at(-1) === 139) break;
  }
  const byName = Object.fromEntries(results.map((row) => [row.name, row]));
  const green = (row) =>
    row &&
    row.statuses.length >= 2 &&
    row.statuses.every((status) => status === 0) &&
    row.exactSelection;
  const measuredBaseline = byName['baseline-measured'];
  const measuredFixed = byName['fixed-measured'];
  const qualification =
    results.every((row) => !row.error) &&
    byName['baseline-exact']?.statuses.at(-1) === 139 &&
    measuredBaseline?.statuses.at(-1) === 139 &&
    green(measuredFixed) &&
    green(byName['fixed-exact']) &&
    green(byName['baseline-file-control']) &&
    green(byName['buffer-overload-control']) &&
    measuredFixed.catalogue.bytes >= measuredBaseline.catalogue.bytes;
  const report = {
    node: process.version,
    cpu: cpus()[0]?.model,
    kernel: release(),
    image: process.env.ImageVersion,
    base: process.env.TMT_BASE_SHA,
    baselineSha256: createHash('sha256').update(old.stdout).digest('hex'),
    fixed: describe(fixedPath),
    tag: draft.tag_name,
    sha: draft.target_commitish,
    qualification,
    results,
    measurement:
      'Exact cases retain the original two-process pipe. Measured cases add tee to seal the entire live producer stream; both old cases must reproduce 139. Byte comparison uses the sealed live streams, not a separate earlier snapshot.',
  };
  writeFileSync(path.join(directory, 'report.json'), JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify(report, null, 2));
  if (!qualification) process.exitCode = 1;
} finally {
  rmSync(baselinePath, { force: true });
  rmSync(bufferPath, { force: true });
}
