// One-shot #1980 hosted qualification. Main tooling owns mutations; reader jobs are read-only.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import {
  createPRRCAPI,
  RC_TARGETS,
  prRCPayloadName,
  prRCCatalogName,
} from './pr-rc-coordinator.mjs';

export const QUALIFICATION = Object.freeze({
  pr: 2715,
  branch: 'infra-1/1980-live-qualification',
  workflow: 380603134,
});
const repository = 'pj-tmt/tmt';
const hash = (bytes) => createHash('sha256').update(bytes).digest('hex');
const MiB = 1024 * 1024;
export function safeEvidence(value) {
  const text = typeof value === 'string' ? value : JSON.stringify(value);
  assert(
    !process.env.GH_TOKEN || !text.includes(process.env.GH_TOKEN),
    'Acquisition token reached evidence.'
  );
  assert(
    !/gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|[?&](?:sig|signature|token|X-Amz-Signature)=/i.test(
      text
    ),
    'Evidence contains a possible credential or signed URL; refusing to retain it.'
  );
  return text;
}
export function producerRunInput(value) {
  assert(typeof value === 'string' && /^[1-9][0-9]{0,15}$/.test(value), 'Invalid producer run ID.');
  const run = Number(value);
  assert(Number.isSafeInteger(run), 'Invalid producer run ID.');
  return run;
}
export async function waitForPull(
  read,
  check,
  sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
) {
  for (let attempt = 0; attempt < 20; attempt++) {
    const { data } = await read();
    assertOwnedPull(data);
    try {
      check(data);
      return data;
    } catch (error) {
      if (attempt === 19) throw error;
      await sleep(3000);
    }
  }
}
export function deriveFixture(run, producer, catalogArtifact, catalog, inventory, pull, timeline) {
  assertOwnedPull(pull);
  assert.equal(producer.id, run);
  assert.equal(producer.workflow_id, QUALIFICATION.workflow);
  assert.equal(producer.path, '.github/workflows/pr-rc.yml');
  assert.equal(producer.run_attempt, 1);
  assert.equal(producer.status, 'completed');
  assert.equal(producer.head_branch, 'main');
  assert.equal(producer.event, 'workflow_dispatch');
  assert.equal(producer.conclusion, 'success');
  assert.equal(producer.repository.full_name, repository);
  assert.equal(catalog.schema_version, 2);
  assert.equal(catalog.kind, 'tmt-pr-rc-catalog');
  assert.equal(catalog.repository, repository);
  assert.equal(catalog.producer.workflow_id, QUALIFICATION.workflow);
  assert.equal(catalog.producer.run_id, run);
  assert.equal(catalog.pr, QUALIFICATION.pr);
  for (const id of [
    catalogArtifact.id,
    catalog.eligibility.label_id,
    catalog.eligibility.enabled_event_id,
  ])
    assert(Number.isSafeInteger(id) && id > 0, 'Invalid fixture identity.');
  assert.match(catalog.head_sha, /^[a-f0-9]{40}$/);
  assert.equal(
    producer.display_title,
    `pr-rc #${QUALIFICATION.pr} ${catalog.head_sha.slice(0, 12)}`
  );
  assert.equal(pull.state, 'open');
  assert.equal(pull.head.sha, catalog.head_sha);
  assert.equal(
    pull.labels.find((label) => label.name === 'rc-build')?.id,
    catalog.eligibility.label_id
  );
  assert(Array.isArray(timeline) && timeline.length < 100);
  const latest = timeline
    .filter((item) => item.event === 'labeled' && item.label?.name === 'rc-build')
    .at(-1);
  assert.equal(latest?.id, catalog.eligibility.enabled_event_id);
  assert.equal(catalogArtifact.name, prRCCatalogName(QUALIFICATION.pr));
  assert.equal(catalogArtifact.workflow_run.id, run);
  assert.deepEqual(catalog.candidates.map((item) => item.target).sort(), [...RC_TARGETS].sort());
  const payloads = catalog.candidates.map((candidate) => candidate.payload_artifact.id);
  assert.equal(new Set([catalogArtifact.id, ...payloads]).size, 5);
  for (const id of payloads) {
    assert(Number.isSafeInteger(id) && id > 0, 'Invalid fixture identity.');
    assert(inventory.some((item) => item.id === id));
  }
  return {
    head: catalog.head_sha,
    run,
    catalog: catalogArtifact.id,
    payloads,
    label: catalog.eligibility.label_id,
    event: catalog.eligibility.enabled_event_id,
  };
}
export function assertOwnedPull(pull) {
  assert.equal(pull.number, QUALIFICATION.pr);
  assert.equal(pull.base?.repo?.full_name, repository);
  assert.equal(pull.head?.repo?.full_name, repository);
  assert.equal(pull.head.ref, QUALIFICATION.branch);
}
export const REFUSAL_MESSAGES = Object.freeze({
  disabled: 'PR is closed, disabled, ineligible or outside the official repository.',
  closed: 'PR is closed, disabled, ineligible or outside the official repository.',
  epoch: 'Catalog producer or eligibility epoch does not match current API evidence.',
  stale: 'PR catalog source, producer or availability evidence does not match.',
});
export function assertRefusal(stage, result, before, after) {
  assert(Object.hasOwn(REFUSAL_MESSAGES, stage), 'Unknown refusal stage.');
  assert(
    Number.isInteger(result.status) && result.status !== 0,
    'Expected a completed nonzero refusal.'
  );
  const report = JSON.parse(result.stdout);
  assert.equal(report.error?.code, 'NATIVE_UPGRADE_FAILED');
  assert.equal(report.error?.message, REFUSAL_MESSAGES[stage]);
  assert.deepEqual(
    after,
    before,
    'Refusal changed installed receipt, links, bytes or application data.'
  );
}
export function assertPositive(result, before, after) {
  assert.equal(result.status, 0, 'Eligible active candidate upgrade must succeed.');
  assert.deepEqual(after, before, 'Eligible re-run changed receipt, prefix or application data.');
}
export function assertPRReceipt(receipt, catalog, fixture) {
  assert.equal(receipt.channel, `pr${QUALIFICATION.pr}`);
  assert.equal(receipt.source?.kind, 'github-actions');
  const proof = receipt.source.evidence;
  assert.equal(proof.repository, repository);
  assert.equal(proof.pr, QUALIFICATION.pr);
  assert.equal(proof.head_sha, fixture.head);
  assert.equal(proof.producer.workflow_id, QUALIFICATION.workflow);
  assert.equal(proof.producer.run_id, fixture.run);
  assert.equal(proof.catalog_artifact_id, fixture.catalog);
  assert.equal(proof.eligibility.label_id, fixture.label);
  assert.equal(proof.eligibility.enabled_event_id, fixture.event);
  const candidate = catalog.candidates.find((item) => item.target === 'x86_64-unknown-linux-musl');
  assert(candidate, 'Matching Linux x86_64 payload missing.');
  assert.equal(receipt.version, candidate.version);
  assert.equal(proof.candidate.payload_artifact.id, candidate.payload_artifact.id);
  assert.equal(receipt.archive_sha256, candidate.archive.sha256);
}
export async function cleanupExact(manifest, call, pull) {
  const fixture = manifest.fixture;
  assertOwnedPull(pull);
  let retained = false;
  if (
    pull.state === 'open' &&
    pull.head.sha === fixture.head &&
    pull.labels.some((label) => label.name === 'rc-build' && label.id === fixture.label)
  ) {
    const timeline = (await call('GET', `issues/${QUALIFICATION.pr}/timeline?per_page=100`)).data;
    assert(Array.isArray(timeline) && timeline.length < 100);
    retained =
      timeline.filter((item) => item.event === 'labeled' && item.label?.name === 'rc-build').at(-1)
        ?.id === fixture.event;
  }
  assert.equal(manifest.run, fixture.run);
  assert.equal(manifest.rc.length, 5);
  assert.deepEqual(
    manifest.rc.map((item) => item.id).sort((a, b) => a - b),
    [fixture.catalog, ...fixture.payloads].sort((a, b) => a - b),
    'Only the frozen five RC IDs may be deleted.'
  );
  assert.equal(manifest.rc[0].id, fixture.catalog, 'Catalog must retire first.');
  if (!retained) {
    if (pull.labels.some((item) => item.name === 'rc-build')) {
      await call('DELETE', `issues/${QUALIFICATION.pr}/labels/rc-build`);
      await waitForPull(
        () => call('GET', `pulls/${QUALIFICATION.pr}`),
        (value) => assert(!value.labels.some((item) => item.name === 'rc-build'))
      );
    }
    if (pull.state === 'closed') {
      await call('PATCH', `pulls/${QUALIFICATION.pr}`, { state: 'open' });
      await waitForPull(
        () => call('GET', `pulls/${QUALIFICATION.pr}`),
        (value) => assert.equal(value.state, 'open')
      );
    }
  }
  const artifacts = [];
  for (const expected of manifest.rc) {
    const route = `actions/artifacts/${expected.id}`;
    const current = await call('GET', route, null, true);
    if (current.status !== 404) {
      for (const key of ['id', 'name', 'size_in_bytes', 'digest'])
        assert.equal(current.data[key], expected[key]);
      assert.equal(current.data.workflow_run.id, fixture.run);
      if (!retained) await call('DELETE', route, null, true);
    }
    if (retained) assert.equal(current.status, 200, 'Unconsumed fixture artifact disappeared.');
    else
      assert.equal(
        (await call('GET', route, null, true)).status,
        404,
        'Exact owned ID is not absent.'
      );
    artifacts.push({ id: expected.id, alreadyAbsent: current.status === 404 });
  }
  for (const id of manifest.ordinary)
    assert.equal(
      (await call('GET', `actions/artifacts/${id}`)).data.id,
      id,
      'Ordinary preparation artifact disappeared.'
    );
  return {
    retained,
    reason: retained
      ? 'PR is open at the captured head with the original rc-build epoch; fixture is unconsumed.'
      : 'Fixture consumed.',
    artifacts,
  };
}

function run(file, args, options = {}) {
  const result = spawnSync(file, args, {
    timeout: 180_000,
    maxBuffer: 128 * MiB,
    encoding: 'utf8',
    ...options,
  });
  assert(
    !result.error && !result.signal,
    `Bounded ${path.basename(file)} command did not complete.`
  );
  safeEvidence(result.stdout);
  safeEvidence(result.stderr);
  return result;
}
function checked(file, args, options) {
  const result = run(file, args, options);
  assert.equal(
    result.status,
    0,
    `Command ${path.basename(file)} failed: ${result.stdout}\n${result.stderr}`
  );
  return result.stdout;
}
export function api(method, route, data = null, absent = false, execute = run) {
  const args = ['api', '--include', '--method', method, `repos/${repository}/${route}`];
  if (data) args.push('--input', '-');
  const result = execute('gh', args, {
    input: data ? JSON.stringify(data) : undefined,
    timeout: 30_000,
    maxBuffer: 2 * MiB,
  });
  safeEvidence(result.stdout);
  const status = /^HTTP\/\S+ (\d+)/.exec(result.stdout);
  assert(status, 'Missing authenticated API status.');
  const code = Number(status[1]);
  const sections = result.stdout.split(/\r?\n\r?\n/);
  const headers = sections.shift();
  const text = sections.join('\n\n').trim();
  if (!((code >= 200 && code < 300) || (absent && code === 404))) {
    let message;
    try {
      message = JSON.parse(text).message;
    } catch {
      /* No JSON message to retain. */
    }
    const permissions = /^x-accepted-github-permissions:[ \t]*([^\r\n]*)/im.exec(headers)?.[1];
    const bounded = (value) =>
      typeof value === 'string'
        ? safeEvidence(value)
            .replace(/[\x00-\x1f\x7f]/g, ' ')
            .slice(0, 200)
        : 'not reported';
    throw new Error(
      `API ${method} failed (${code}): ${bounded(message)}; x-accepted-github-permissions: ${bounded(permissions)}`
    );
  }
  return { status: code, data: text ? JSON.parse(text) : null };
}

function snapshot(directory) {
  const result = {};
  const visit = (root, relative = '') => {
    for (const entry of fs
      .readdirSync(root, { withFileTypes: true })
      .sort((a, b) => a.name.localeCompare(b.name))) {
      const name = path.join(relative, entry.name);
      const file = path.join(root, entry.name);
      if (entry.isSymbolicLink()) result[name] = { link: fs.readlinkSync(file) };
      else if (entry.isDirectory()) visit(file, name);
      else {
        assert(entry.isFile());
        result[name] = { sha256: hash(fs.readFileSync(file)) };
      }
    }
  };
  visit(directory);
  return result;
}
export function readOwnedReceipt(world) {
  const prefix = fs.realpathSync(path.join(world, 'prefix'));
  const active = fs.realpathSync(path.join(prefix, 'lib/tmux-team/current'));
  assert(active.startsWith(`${prefix}/`), 'Active receipt escapes owned prefix.');
  return JSON.parse(safeEvidence(fs.readFileSync(path.join(active, 'receipt.json'), 'utf8')));
}
function clientEnvironment(world) {
  // Deliberately construct the environment: no host ancestry, providers, legacy home or token alias.
  return {
    PATH: `${path.join(world, 'prefix/bin')}:/usr/local/bin:/usr/bin:/bin`,
    LANG: 'C.UTF-8',
    HOME: path.join(world, 'home'),
    TMT_HOME: path.join(world, 'state'),
    TMPDIR: path.join(world, 'tmp'),
    XDG_CONFIG_HOME: path.join(world, 'config'),
    XDG_DATA_HOME: path.join(world, 'data'),
    XDG_STATE_HOME: path.join(world, 'xdg-state'),
    XDG_CACHE_HOME: path.join(world, 'cache'),
    CODEX_HOME: path.join(world, 'codex'),
    TMT_AGENT: '1',
    BROWSER: '/usr/bin/true',
    GH_TOKEN: process.env.GH_TOKEN,
  };
}
function binaryIdentity(world, role) {
  const program = path.join(world, 'prefix/bin/tmt');
  const installed = readOwnedReceipt(world);
  const version = checked(program, ['--version'], { env: clientEnvironment(world) }).trim();
  assert.equal(version, installed.version, 'Running binary and receipt versions differ.');
  return {
    role,
    path: program,
    realpath: fs.realpathSync(program),
    version,
    sha256: hash(fs.readFileSync(program)),
  };
}
function alphaOrder(a, b) {
  const parse = (version) => {
    const match = /^(\d+)\.(\d+)\.(\d+)-alpha\.(\d+)$/.exec(version);
    assert(match, 'Well-formed alpha version required.');
    return match.slice(1).map(Number);
  };
  const left = parse(a),
    right = parse(b);
  for (let i = 0; i < left.length; i++)
    if (left[i] !== right[i]) return Math.sign(left[i] - right[i]);
  return 0;
}
export async function executeSelftest(stage) {
  assert.equal(process.env.GITHUB_REPOSITORY, repository);
  assert.equal(process.env.GITHUB_REF, 'refs/heads/main', 'Selftest requires protected main.');
  assert.equal(process.env.GITHUB_EVENT_NAME, 'workflow_dispatch');
  assert(process.env.GH_TOKEN, 'Environment token required.');
  const root = path.join(process.env.RUNNER_TEMP, 'pr-rc-selftest');
  const evidence = path.join(root, 'evidence');
  fs.mkdirSync(evidence, { recursive: true });
  const save = (name, value) =>
    fs.writeFileSync(path.join(evidence, `${name}.json`), `${safeEvidence(value)}\n`);
  if (stage === 'seal') {
    let total = 0;
    const visit = (directory) => {
      for (const item of fs.readdirSync(directory, { withFileTypes: true })) {
        const file = path.join(directory, item.name);
        if (item.isDirectory()) visit(file);
        else if (item.isFile()) {
          const bytes = fs.readFileSync(file);
          total += bytes.length;
          assert(total <= 256 * MiB, 'Isolated world exceeds capture bound.');
          assert(
            !process.env.GH_TOKEN || !bytes.includes(Buffer.from(process.env.GH_TOKEN)),
            'Acquisition token persisted in isolated world.'
          );
          assert(
            !/gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}/.test(
              bytes.toString('latin1')
            ),
            'Token-like bytes in isolated world.'
          );
          if (item.name.endsWith('.json') || item.name === 'sentinel')
            safeEvidence(bytes.toString('utf8'));
        } else assert(item.isSymbolicLink(), 'Special file in captured world.');
      }
    };
    visit(path.join(root, 'world'));
    return;
  }
  if (stage === 'scan') {
    for (const name of fs.readdirSync(evidence))
      safeEvidence(fs.readFileSync(path.join(evidence, name), 'utf8'));
    return;
  }
  const manifestFile = path.join(root, 'manifest.json');
  const pull = () => {
    const value = api('GET', `pulls/${QUALIFICATION.pr}`).data;
    assertOwnedPull(value);
    return value;
  };
  if (stage === 'preflight') {
    const utc = new Date();
    assert(
      !(utc.getUTCHours() === 4 && utc.getUTCMinutes() >= 31 && utc.getUTCMinutes() <= 51),
      'Avoid the daily cleanup window (04:41Z +/-10min).'
    );
    const current = pull();
    const run = producerRunInput(process.env.PR_RC_SELFTEST_RUN);
    const reader = createPRRCAPI();
    const producer = reader.metadata(`actions/runs/${run}`);
    const inventory = reader.inventory(run);
    const catalogs = inventory.filter((item) => item.name === prRCCatalogName(QUALIFICATION.pr));
    assert.equal(catalogs.length, 1, 'Exactly one catalog required.');
    const catalogArtifact = catalogs[0];
    const catalogMembers = reader.members(catalogArtifact, 2 * MiB);
    assert.equal(catalogMembers.length, 1);
    assert.equal(catalogMembers[0].name, 'catalog.json');
    const catalog = JSON.parse(catalogMembers[0].bytes);
    const timeline = api('GET', `issues/${QUALIFICATION.pr}/timeline?per_page=100`).data;
    const fixture = deriveFixture(
      run,
      producer,
      catalogArtifact,
      catalog,
      inventory,
      current,
      timeline
    );
    const rc = [catalogArtifact];
    for (const candidate of catalog.candidates) {
      const artifact = inventory.find((item) => item.id === candidate.payload_artifact.id);
      assert(artifact);
      assert.equal(artifact.name, candidate.payload_artifact.name);
      assert.equal(
        artifact.name,
        prRCPayloadName(QUALIFICATION.pr, candidate.target, fixture.run, 1)
      );
      assert.equal(artifact.workflow_run.id, fixture.run);
      assert.equal(artifact.size_in_bytes, candidate.payload_artifact.zip_bytes);
      assert.equal(artifact.digest, `sha256:${candidate.payload_artifact.zip_sha256}`);
      assert(Date.parse(artifact.created_at) <= Date.parse(catalogArtifact.created_at));
      const members = reader.members(artifact, 69 * MiB);
      assert.equal(members.length, 2);
      for (const expected of [candidate.dist_manifest, candidate.archive]) {
        const member = members.find((item) => item.name === expected.name);
        assert(member);
        assert.equal(member.bytes.length, expected.bytes);
        assert.equal(hash(member.bytes), expected.sha256);
      }
      rc.push(artifact);
    }
    const manifest = {
      run: fixture.run,
      fixture,
      catalog,
      rc: rc.map(({ id, name, size_in_bytes, digest }) => ({ id, name, size_in_bytes, digest })),
      ordinary: inventory.filter((item) => !rc.includes(item)).map((item) => item.id),
    };
    fs.writeFileSync(manifestFile, `${safeEvidence(manifest)}\n`);
    save(stage, manifest);
    return;
  }
  const manifest = JSON.parse(fs.readFileSync(manifestFile, 'utf8'));
  const fixture = manifest.fixture;
  const world = path.join(root, 'world');
  if (stage === 'baseline') {
    assert(!fs.existsSync(world), 'Fresh isolated world required.');
    for (const name of ['home', 'state', 'tmp', 'config', 'data', 'xdg-state', 'cache', 'codex'])
      fs.mkdirSync(path.join(world, name), { recursive: true });
    fs.writeFileSync(
      path.join(world, 'state/sentinel'),
      '#1980 application data must remain unchanged\n'
    );
    const env = clientEnvironment(world);
    delete env.GH_TOKEN;
    const installer = checked('curl', [
      '--fail',
      '--silent',
      '--show-error',
      '--location',
      '--max-time',
      '120',
      '--max-filesize',
      String(MiB),
      'https://github.com/pj-tmt/tmt/releases/latest/download/install.sh',
    ]);
    const version = /^\s*version='([^']+)'$/m.exec(installer)?.[1];
    assert(version);
    const candidate = manifest.catalog.candidates.find(
      (item) => item.target === 'x86_64-unknown-linux-musl'
    );
    assert(candidate);
    assert(
      alphaOrder(version, candidate.version) <= 0,
      'Published client is newer than frozen candidate; a fresh producer generation is needed.'
    );
    checked(
      '/bin/sh',
      ['-s', '--', '--prefix', path.join(world, 'prefix'), '--no-skill', '--no-setup'],
      { input: installer, env }
    );
    const executable = path.join(world, 'prefix/bin/tmt');
    checked(executable, ['identity', 'ls', '--json'], { env });
    const dataBefore = snapshot(path.join(world, 'state'));
    const baseline = readOwnedReceipt(world);
    assert.equal(baseline.version, version);
    assert.equal(baseline.channel, 'alpha');
    const binaryRan = binaryIdentity(world, 'published-client');
    const output = checked(
      executable,
      ['upgrade', '--channel', `pr${QUALIFICATION.pr}`, '--yes', '--json'],
      { env: clientEnvironment(world) }
    );
    assertPRReceipt(readOwnedReceipt(world), manifest.catalog, fixture);
    assert.deepEqual(
      snapshot(path.join(world, 'state')),
      dataBefore,
      'PR install modified application data.'
    );
    const positiveBefore = {
      receipt: readOwnedReceipt(world),
      prefix: snapshot(path.join(world, 'prefix')),
      data: snapshot(path.join(world, 'state')),
    };
    const positiveBinary = binaryIdentity(world, 'active-candidate');
    const positive = run(
      executable,
      ['upgrade', '--channel', `pr${QUALIFICATION.pr}`, '--yes', '--json'],
      { env: clientEnvironment(world) }
    );
    save('positive', {
      binaryRan: positiveBinary,
      status: positive.status,
      stdout: positive.stdout,
      stderr: positive.stderr,
    });
    assertPositive(positive, positiveBefore, {
      receipt: readOwnedReceipt(world),
      prefix: snapshot(path.join(world, 'prefix')),
      data: snapshot(path.join(world, 'state')),
    });
    fs.writeFileSync(
      path.join(world, 'frozen.json'),
      `${safeEvidence({ root: world, dataBefore, state: snapshot(path.join(world, 'prefix')), receipt: readOwnedReceipt(world) })}\n`
    );
    save(stage, {
      version,
      binaryRan,
      installerSha256: hash(installer),
      report: JSON.parse(output),
      receipt: readOwnedReceipt(world),
      target: 'x86_64-unknown-linux-musl',
    });
    return;
  }
  if (['stale', 'disabled', 'epoch', 'closed'].includes(stage)) {
    const current = pull();
    if (stage === 'stale') assert.notEqual(current.head.sha, fixture.head);
    if (stage === 'disabled') assert(!current.labels.some((item) => item.name === 'rc-build'));
    if (stage === 'epoch') {
      assert.equal(current.head.sha, fixture.head);
      assert(current.labels.some((item) => item.name === 'rc-build'));
      const timeline = api('GET', `issues/${QUALIFICATION.pr}/timeline?per_page=100`).data;
      assert(timeline.length < 100);
      assert.notEqual(
        timeline
          .filter((item) => item.event === 'labeled' && item.label?.name === 'rc-build')
          .at(-1)?.id,
        fixture.event
      );
    }
    if (stage === 'closed') assert.equal(current.state, 'closed');
    const frozen = JSON.parse(fs.readFileSync(path.join(world, 'frozen.json'), 'utf8'));
    assert.equal(frozen.root, world, 'World restored at a different absolute prefix.');
    const before = {
      prefix: snapshot(path.join(world, 'prefix')),
      data: snapshot(path.join(world, 'state')),
    };
    assert.deepEqual(before.prefix, frozen.state);
    assert.deepEqual(before.data, frozen.dataBefore);
    const binaryRan = binaryIdentity(world, 'active-candidate');
    const result = run(
      path.join(world, 'prefix/bin/tmt'),
      ['upgrade', '--channel', `pr${QUALIFICATION.pr}`, '--yes', '--json'],
      { env: clientEnvironment(world) }
    );
    assertRefusal(stage, result, before, {
      prefix: snapshot(path.join(world, 'prefix')),
      data: snapshot(path.join(world, 'state')),
    });
    save(stage, {
      binaryRan,
      status: result.status,
      stdout: result.stdout,
      stderr: result.stderr,
      unchanged: true,
    });
    if (stage === 'closed') {
      const binaryRan = binaryIdentity(world, 'active-candidate');
      const output = checked(
        path.join(world, 'prefix/bin/tmt'),
        ['upgrade', '--channel', 'alpha', '--yes', '--json'],
        { env: clientEnvironment(world) }
      );
      const normal = readOwnedReceipt(world);
      assert.equal(normal.channel, 'alpha');
      assert.notEqual(normal.source?.kind, 'github-actions');
      assert.deepEqual(
        snapshot(path.join(world, 'state')),
        frozen.dataBefore,
        'Alpha return changed application data.'
      );
      save('normal', { binaryRan, report: JSON.parse(output), receipt: normal });
    }
    return;
  }
  const current = pull();
  if (stage === 'advance') {
    assert.equal(current.state, 'open');
    assert.equal(current.head.sha, fixture.head);
    const parent = api('GET', `git/commits/${fixture.head}`).data;
    const commit = api('POST', 'git/commits', {
      message:
        'docs(infra): #1980 stale-head refusal (never merge)\n\nCo-authored-by: Codex <codex@openai.com>',
      tree: parent.tree.sha,
      parents: [fixture.head],
    }).data;
    api('PATCH', `git/refs/heads/${QUALIFICATION.branch}`, { sha: commit.sha, force: false });
    await waitForPull(
      () => ({ data: pull() }),
      (value) => assert.equal(value.head.sha, commit.sha)
    );
    save(stage, { head: commit.sha });
  } else if (stage === 'disable') {
    api('DELETE', `issues/${QUALIFICATION.pr}/labels/rc-build`);
    await waitForPull(
      () => ({ data: pull() }),
      (value) => assert(!value.labels.some((item) => item.name === 'rc-build'))
    );
    save(stage, { disabled: true });
  } else if (stage === 'enable') {
    api('POST', `issues/${QUALIFICATION.pr}/labels`, { labels: ['rc-build'] });
    await waitForPull(
      () => ({ data: pull() }),
      (value) =>
        assert.equal(value.labels.find((item) => item.name === 'rc-build')?.id, fixture.label)
    );
    save(stage, { enabled: true });
  } else if (stage === 'close') {
    api('PATCH', `pulls/${QUALIFICATION.pr}`, { state: 'closed' });
    await waitForPull(
      () => ({ data: pull() }),
      (value) => assert.equal(value.state, 'closed')
    );
    save(stage, { closed: true });
  } else if (stage === 'cleanup') {
    save(stage, {
      ...(await cleanupExact(manifest, api, current)),
      closeEventCleanupExercised: false,
    });
  } else throw new Error('Unknown selftest stage.');
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  executeSelftest(process.argv[2])
    .then(() => console.log('PR RC selftest stage complete.'))
    .catch((error) => {
      // Never print an exception containing an acquisition URL/token. The stage evidence is scanned separately.
      try {
        console.error(safeEvidence(error.message));
      } catch {
        console.error('PR RC selftest failed with unsafe diagnostic; evidence withheld.');
      }
      process.exitCode = 1;
    });
}
