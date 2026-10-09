import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { GATES } from './publication-gates.mjs';

const ROOTS = ['native-release.yml', 'release.yml'];
// Every failed or held release closes its gap here: an incident row names the release step that
// caught it and either a pre-merge counterpart or a concrete release-only reason.
const INCIDENTS = [
  1534, 1541, 1542, 1550, 1593, 1604, 1616, 1643, 1646, 1661, 1680, 1745, 2076, 2197, 2241,
];
const repository = fileURLToPath(new URL('../../', import.meta.url));
const digest = (text) => createHash('sha256').update(text).digest('hex');
const nonempty = (value) => typeof value === 'string' && value.trim().length > 0;
const record = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);

function ensure(condition, message) {
  if (!condition) throw new Error(message);
}

/** Admit the existing block-form workflow convention; unsupported structure fails closed. */
export function inventoryWorkflow(text, file, selectedJob) {
  const sections = text.split(/^jobs:\s*$/m);
  ensure(sections.length === 2, `${file}: expected one block-form jobs mapping`);
  const jobs = Object.create(null);
  let current;
  for (const line of sections[1].split('\n')) {
    if (/^\s*(?:#.*)?$/.test(line)) continue;
    if (/^\S/.test(line)) throw new Error(`${file}: unsupported root after jobs`);
    const declaration = /^  ([A-Za-z_][\w-]*):\s*(?:#.*)?$/.exec(line);
    if (declaration) {
      current = declaration[1];
      ensure(!Object.hasOwn(jobs, current), `${file}: duplicate job ${current}`);
      jobs[current] = { lines: [] };
    } else {
      ensure(current && /^ {4}\S|^ {5,}/.test(line), `${file}: unsupported job declaration`);
      jobs[current].lines.push(line);
    }
  }
  ensure(Object.keys(jobs).length > 0, `${file}: no release jobs discovered`);
  return Object.fromEntries(
    Object.entries(jobs)
      .filter(([name]) => !selectedJob || name === selectedJob)
      .map(([name, { lines }]) => {
        const body = lines.join('\n');
        const label = `${file}:${name}`;
        const calls = [...body.matchAll(/^ {4}uses:\s*(\S+)\s*(?:#.*)?$/gm)];
        const stepMappings = [...body.matchAll(/^ {4}steps:\s*$/gm)];
        ensure(
          calls.length + stepMappings.length === 1,
          `${label}: expected steps or one workflow call`
        );
        const steps = [];
        let uses;
        if (calls.length) {
          uses = calls[0][1];
          ensure(
            /^\.\/\.github\/workflows\/[\w-]+\.yml$/.test(uses),
            `${label}: unsupported reusable workflow ${uses}`
          );
          steps.push(`uses:${uses}`);
        } else {
          const stepBody = body.slice(stepMappings[0].index + stepMappings[0][0].length);
          ensure(/^\s*(?:#.*\n\s*)* {6}- /m.test(stepBody), `${label}: expected block-form steps`);
          const blocks = stepBody.split(/\n(?= {6}- )/).filter((block) => /^ {6}- /m.test(block));
          const occurrences = new Map();
          for (const block of blocks) {
            ensure(
              /^ {6}- (?:name|id|uses|run|if|env|with|working-directory|shell|continue-on-error|timeout-minutes):/.test(
                block
              ),
              `${label}: unsupported step mapping`
            );
            const fields = (key) => [
              ...block.matchAll(new RegExp(`^(?: {6}- | {8})${key}:\\s*(.+)$`, 'gm')),
            ];
            const execution = [...fields('uses'), ...fields('run')];
            ensure(execution.length === 1, `${label}: each step needs exactly one run or uses`);
            ensure(
              fields('id').length <= 1 && fields('name').length <= 1,
              `${label}: duplicate step identity`
            );
            const id = fields('id')[0]?.[1];
            const title = fields('name')[0]?.[1];
            const key = id
              ? `id:${id}`
              : title
                ? `name:${title}`
                : `uses:${fields('uses')[0]?.[1]}`;
            ensure(!key.endsWith('undefined'), `${label}: run steps need an id or name`);
            const count = (occurrences.get(key) ?? 0) + 1;
            occurrences.set(key, count);
            ensure(count === 1 || key.startsWith('uses:'), `${label}: duplicate step ${key}`);
            steps.push(count === 1 ? key : `${key}#${count}`);
          }
          ensure(steps.length > 0, `${label}: empty step inventory`);
        }
        // Includes commands, conditions, matrix/target selection and inputs: a new command inside
        // an existing named step also demands review, rather than silently inheriting its mapping.
        return [name, { steps, sha256: digest(body), uses, body }];
      })
  );
}

export function releaseInventory(read) {
  const workflows = Object.create(null);
  const visit = (file) => {
    if (Object.hasOwn(workflows, file)) return;
    const jobs = inventoryWorkflow(read(`.github/workflows/${file}`), file);
    workflows[file] = jobs;
    for (const job of Object.values(jobs)) {
      if (job.uses) visit(path.posix.basename(job.uses));
    }
  };
  ROOTS.forEach(visit);
  return workflows;
}

function sameKeys(actual, expected, label) {
  ensure(record(expected), `${label}: expected a mapping`);
  for (const key of Object.keys(actual))
    ensure(Object.hasOwn(expected, key), `${label}: unmapped ${key}`);
  for (const key of Object.keys(expected))
    ensure(Object.hasOwn(actual, key), `${label}: stale ${key}`);
}

function checkCoverage(entry, label, read) {
  ensure(record(entry), `${label}: expected a coverage record`);
  ensure(
    Object.hasOwn(entry, 'preMerge') !== Object.hasOwn(entry, 'releaseOnly'),
    `${label}: choose preMerge or releaseOnly`
  );
  if (Object.hasOwn(entry, 'releaseOnly')) {
    ensure(nonempty(entry.releaseOnly), `${label}: releaseOnly needs a reason`);
    ensure(
      !Object.hasOwn(entry, 'followUp'),
      `${label}: a release-only entry cannot keep a follow-up`
    );
    return;
  }
  ensure(
    Array.isArray(entry.preMerge) && entry.preMerge.length > 0,
    `${label}: empty preMerge coverage`
  );
  for (const counterpart of entry.preMerge) {
    ensure(
      record(counterpart) &&
        /^[\w-]+\.yml$/.test(counterpart.workflow) &&
        nonempty(counterpart.job),
      `${label}: invalid preMerge reference`
    );
    const source = read(`.github/workflows/${counterpart.workflow}`);
    const job = inventoryWorkflow(source, counterpart.workflow, counterpart.job)[counterpart.job];
    ensure(job, `${label}: missing preMerge job ${counterpart.workflow}:${counterpart.job}`);
    const selection = counterpart.selection;
    ensure(record(selection), `${label}: missing path selection`);
    ensure(['ci-scope', 'always'].includes(selection.kind), `${label}: unknown selection kind`);
    if (selection.kind === 'always') {
      // An always-run counterpart is selected by no path: the job runs on every verification event,
      // so its `if` may name the event class (`verify`) and nothing a changed path decides.
      const condition = job.body.match(/^ {4}if:\s*(.+)$/m)?.[1] ?? '';
      ensure(
        !/needs\.changes\.outputs\.(?!verify\b)/.test(condition),
        `${label}: an always-run job must not depend on a path selection`
      );
      ensure(
        nonempty(selection.step) && job.steps.includes(selection.step),
        `${label}: always-run counterpart needs a real step of ${counterpart.job}`
      );
    } else {
      ensure(
        nonempty(selection.output) && nonempty(selection.value),
        `${label}: invalid ci-scope selector`
      );
      const condition = `needs.changes.outputs.${selection.output} == '${selection.value}'`;
      const jobCondition = job.body.match(/^ {4}if:\s*(.+)$/m)?.[1];
      ensure(jobCondition?.includes(condition), `${label}: preMerge job does not use ${condition}`);
      ensure(
        source.includes(`      ${selection.output}:`),
        `${label}: missing ci-scope output ${selection.output}`
      );
      // ci-scope emits most selections; a selector with its own module names it as `source`.
      const emitter = selection.source ?? 'typescript/scripts/ci-scope.mjs';
      ensure(
        /^typescript\/scripts\/[\w-]+\.mjs$/.test(emitter),
        `${label}: invalid ci-scope selector source`
      );
      ensure(
        read(emitter).includes(`${selection.output}=`),
        `${label}: ${emitter} does not emit ${selection.output}`
      );
    }
    ensure(
      ['runtime', 'policy'].includes(counterpart.coverage),
      `${label}: unknown counterpart coverage`
    );
    if (counterpart.coverage === 'policy') {
      ensure(
        Array.isArray(counterpart.tests) && counterpart.tests.length > 0,
        `${label}: policy coverage needs tests`
      );
      ensure(
        /^ {8,}pnpm test:run\s*$/m.test(job.body),
        `${label}: policy job does not run the tooling suite`
      );
      for (const test of counterpart.tests) {
        ensure(
          /^typescript\/test\/tooling\/[\w-]+\.test\.ts$/.test(test),
          `${label}: invalid test path`
        );
        read(test);
      }
    }
  }
}

/** The manifest never creates coverage: it records existing jobs and explicit remaining gaps. */
export function checkReleaseParity(manifest, { read, publicationGates = GATES }) {
  ensure(record(manifest) && manifest.schemaVersion === 1, 'Unsupported release-parity schema');
  const inventory = releaseInventory(read);
  sameKeys(inventory, manifest.workflows, 'release workflows');
  let jobs = 0;
  for (const [file, definitions] of Object.entries(inventory)) {
    sameKeys(definitions, manifest.workflows[file], file);
    for (const [name, definition] of Object.entries(definitions)) {
      const label = `${file}:${name}`;
      const entry = manifest.workflows[file][name];
      ensure(record(entry), `${label}: expected a job record`);
      ensure(
        JSON.stringify(entry.steps) === JSON.stringify(definition.steps),
        `${label}: executable step inventory changed`
      );
      ensure(
        entry.sha256 === definition.sha256,
        `${label}: executable definition changed; review coverage before updating its fingerprint`
      );
      checkCoverage(entry, label, read);
      jobs += 1;
    }
  }
  sameKeys(
    Object.fromEntries(publicationGates.map((gate) => [gate, true])),
    manifest.publicationGates,
    'publication gates'
  );
  for (const [gate, entry] of Object.entries(manifest.publicationGates))
    checkCoverage(entry, `publication:${gate}`, read);
  sameKeys(
    Object.fromEntries(INCIDENTS.map((issue) => [issue, true])),
    manifest.incidents,
    'release incidents'
  );
  for (const [issue, entry] of Object.entries(manifest.incidents)) {
    const source = entry.release;
    const job = inventory[source?.workflow]?.[source?.job];
    ensure(job?.steps.includes(source?.step), `incident #${issue}: missing release step`);
    checkCoverage(entry, `incident #${issue}`, read);
  }
  return {
    workflows: Object.keys(inventory).length,
    jobs,
    publicationGates: publicationGates.length,
  };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const read = (file) => readFileSync(path.join(repository, file), 'utf8');
  const args = process.argv.slice(2);
  ensure(
    args.length <= 1 && ['check', 'inventory'].includes(args[0] ?? 'check'),
    'Usage: release-parity.mjs [check|inventory]'
  );
  const result =
    args[0] === 'inventory'
      ? Object.fromEntries(
          Object.entries(releaseInventory(read)).map(([file, jobs]) => [
            file,
            Object.fromEntries(
              Object.entries(jobs).map(([name, { steps, sha256 }]) => [name, { steps, sha256 }])
            ),
          ])
        )
      : checkReleaseParity(JSON.parse(read('.github/release-parity.json')), { read });
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}
