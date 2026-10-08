import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vite-plus/test';

const script = fileURLToPath(new URL('../../scripts/release-publish.mjs', import.meta.url));
const componentMap = fileURLToPath(new URL('../../../.github/components.json', import.meta.url));

const SHA = 'a'.repeat(40);
const TAG = 'v5.0.0-alpha.9';
const ASSETS = ['release-publication.json', 'dist-manifest.json', 'tmt-cli-x.tar.gz'];

let root: string;
beforeAll(() => {
  root = mkdtempSync(path.join(os.tmpdir(), 'release-publish-'));
});
afterAll(() => rmSync(root, { recursive: true, force: true }));

/** A fake `gh` that answers from a state file and records every call. */
const FAKE_GH = `#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');
const state = JSON.parse(fs.readFileSync(process.env.FAKE_GH_STATE, 'utf8'));
const args = process.argv.slice(2);
fs.appendFileSync(state.calls, JSON.stringify(args) + '\\n');
const out = (value) => process.stdout.write(typeof value === 'string' ? value : JSON.stringify(value));
const fail = (message) => { process.stderr.write(message); process.exit(1); };
const [command, sub] = args;
if (command === 'api') {
  const url = args[1];
  if (args.includes('--paginate')) out([state.drafts.length ? state.drafts : state.published ? [state.published] : []]);
  else if (url.includes('/releases/tags/')) state.published ? out(state.published) : fail('HTTP 404');
  else if (url.includes('/releases/') && args.includes('PATCH')) { state.latest = state.published; fs.writeFileSync(process.env.FAKE_GH_STATE, JSON.stringify(state)); out(state.latest); }
  else if (url.endsWith('/releases/latest')) state.latest ? out(state.latest) : fail('HTTP 404');
  else if (url.includes('/commits/')) state.tagCommit ? out({ sha: state.tagCommit }) : fail('HTTP 404');
  else if (url.includes('/issues?state=open')) out(state.openIssues ?? []);
  else if (url.endsWith('/issues') && args.includes('POST')) {
    if (state.issueFails) fail('HTTP 403');
    out({ number: 77 });
  }
  else if (url.endsWith('/comments') && args.includes('POST')) out({ id: 1 });
  else fail('unexpected api call: ' + url);
} else if (command === 'release' && sub === 'edit') {
  if (state.editFails) fail('HTTP 422');
  const tag = args[2];
  state.drafts = state.drafts.map(release => release.tag_name === tag ? { ...release, draft: false } : release);
  state.published = state.drafts.find(release => release.tag_name === tag);
  fs.writeFileSync(process.env.FAKE_GH_STATE, JSON.stringify(state));
} else if (command === 'release' && sub === 'download') {
  if (state.downloadFails) fail('HTTP 502');
  const directory = args[args.indexOf('--dir') + 1];
  for (const name of state.assetNames) fs.writeFileSync(path.join(directory, name), name);
} else if (command === 'release' && sub === 'verify') {
  if (state.attestationFails) fail('attestation not found');
} else if (command === 'release' && sub === 'verify-asset') {
  if ((state.badAssets ?? []).includes(path.basename(args[3]))) fail('does not contain subject');
} else {
  fail('unexpected gh call: ' + args.join(' '));
}
`;

const release = (overrides: Record<string, unknown> = {}) => ({
  id: 1,
  draft: false,
  immutable: true,
  prerelease: false,
  tag_name: TAG,
  target_commitish: SHA,
  created_at: '2026-09-30T01:00:00Z',
  assets: ASSETS.map((name, index) => ({ id: index + 1, name })),
  ...overrides,
});

interface Scenario {
  drafts?: unknown[];
  published?: unknown;
  latest?: unknown;
  tagCommit?: string | null;
  editFails?: boolean;
  downloadFails?: boolean;
  attestationFails?: boolean;
  badAssets?: string[];
  openIssues?: { number: number; title: string }[];
  issueFails?: boolean;
}

function scenario(options: Scenario = {}) {
  const directory = mkdtempSync(path.join(root, 'case-'));
  const bin = path.join(directory, 'bin');
  mkdirSync(bin);
  const ghFile = path.join(bin, 'gh');
  writeExecutable(ghFile, FAKE_GH, 0o755);
  const calls = path.join(directory, 'calls');
  writeFileSync(calls, '');
  const stateFile = path.join(directory, 'state.json');
  writeFileSync(
    stateFile,
    JSON.stringify({
      calls,
      assetNames: ASSETS,
      drafts: [],
      published: release(),
      latest: release(),
      tagCommit: SHA,
      ...options,
    })
  );
  const output = path.join(directory, 'output');
  const summary = path.join(directory, 'summary');
  writeFileSync(output, '');
  writeFileSync(summary, '');
  return {
    directory,
    run(args: string[]) {
      const result = spawnSync('node', [script, ...args], {
        encoding: 'utf8',
        env: {
          PATH: `${bin}${path.delimiter}${process.env.PATH}`,
          GITHUB_REPOSITORY: 'wkh237/tmt',
          GITHUB_SERVER_URL: 'https://github.com',
          GITHUB_RUN_ID: '42',
          GITHUB_OUTPUT: output,
          GITHUB_STEP_SUMMARY: summary,
          FAKE_GH_STATE: stateFile,
        },
        timeout: 30_000,
      });
      return {
        status: result.status,
        stderr: result.stderr,
        output: readFileSync(output, 'utf8'),
        summary: readFileSync(summary, 'utf8'),
      };
    },
    calls: () =>
      readFileSync(calls, 'utf8')
        .split('\n')
        .filter(Boolean)
        .map((line) => JSON.parse(line) as string[]),
  };
}

const draft = (tag: string, names: string[], overrides: Record<string, unknown> = {}) =>
  release({
    tag_name: tag,
    draft: true,
    immutable: false,
    assets: names.map((name, index) => ({ id: index + 1, name })),
    ...overrides,
  });

describe('release-publish.mjs publish', () => {
  const publish = ['publish', '--product', 'cli', '--tag', TAG];

  it('publishes a complete draft with one gh release edit carrying the CLI policy', () => {
    const { run, calls } = scenario({ drafts: [draft(TAG, ASSETS)] });
    const result = run(publish);
    expect(result.status).toBe(0);
    expect(result.output).toBe('published=true\n');
    expect(result.summary).toContain('--draft=false --prerelease=false --latest=false');
    expect(calls().filter(([command, sub]) => command === 'release' && sub === 'edit')).toEqual([
      [
        'release',
        'edit',
        TAG,
        '--repo',
        'wkh237/tmt',
        '--draft=false',
        '--prerelease=false',
        '--latest=false',
      ],
    ]);
  });

  it('publishes an extension as a prerelease that is never the latest release', () => {
    const tag = 'tmt-ops-v0.1.0-alpha.2';
    const { run, calls, directory } = scenario({ drafts: [draft(tag, ASSETS)] });
    expect(run(['publish', '--product', 'ops', '--tag', tag]).status).toBe(1);
    expect(calls().some(([, sub]) => sub === 'edit')).toBe(false);
    const map = JSON.parse(readFileSync(componentMap, 'utf8'));
    map.components.ops.release = true; // Isolated activation fixture, never the committed map.
    const activated = path.join(directory, 'activated-components.json');
    writeFileSync(activated, JSON.stringify(map));
    expect(
      run(['publish', '--product', 'ops', '--tag', tag, '--components', activated]).status
    ).toBe(0);
    expect(calls().find(([, sub]) => sub === 'edit')).toEqual([
      'release',
      'edit',
      tag,
      '--repo',
      'wkh237/tmt',
      '--draft=false',
      '--prerelease=true',
      '--latest=false',
    ]);
  });

  it.each([
    ['is held', [...ASSETS, 'publication-held.json'], 'is held'],
    ['has a recorded failure', [...ASSETS, 'verification-failed.json'], 'recorded failure'],
    ['has no bundle', ['dist-manifest.json'], 'no verified bundle'],
  ])('refuses, and runs no gh release edit, for a draft that %s', (_name, names, message) => {
    const { run, calls } = scenario({ drafts: [draft(TAG, names)] });
    const result = run(publish);
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('Not publishing:');
    expect(result.stderr).toContain(message);
    expect(result.output).toBe('');
    expect(calls().some(([, sub]) => sub === 'edit')).toBe(false);
  });

  it('refuses a draft of another product and an unknown draft', () => {
    const { run, calls } = scenario({ drafts: [draft(TAG, ASSETS)] });
    expect(run(['publish', '--product', 'office', '--tag', TAG]).stderr).toContain(
      'is not a office release tag'
    );
    expect(run(['publish', '--product', 'cli', '--tag', 'v5.0.0-alpha.99']).stderr).toContain(
      'there is no release v5.0.0-alpha.99'
    );
    expect(calls().some(([, sub]) => sub === 'edit')).toBe(false);
  });

  it.each(['v5.0.0', 'v5.0.0-beta.1', 'v5.0.0-rc.1'])(
    'never publishes the release %s, which is not an alpha, however complete its draft',
    (tag) => {
      const { run, calls } = scenario({ drafts: [draft(tag, ASSETS)] });
      const result = run(['publish', '--product', 'cli', '--tag', tag]);
      expect(result.status).toBe(1);
      expect(result.stderr).toContain(`Not publishing: ${tag} is not an alpha release`);
      expect(result.output).toBe('');
      expect(calls().some(([, sub]) => sub === 'edit')).toBe(false);
    }
  );

  it('never publishes a component that the component map does not release', () => {
    const fake = scenario({ drafts: [draft(TAG, ASSETS)] });
    const map = JSON.parse(readFileSync(componentMap, 'utf8')) as {
      components: Record<string, { release?: boolean }>;
    };
    map.components.cli.release = false;
    const parked = path.join(fake.directory, 'components.json');
    writeFileSync(parked, JSON.stringify(map));
    const result = fake.run([...publish, '--components', parked]);
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('cli is not released (release: false');
    expect(fake.calls().some(([, sub]) => sub === 'edit')).toBe(false);
    // The committed map releases it, which the other tests rely on.
    expect(fake.run(publish).status).toBe(0);
  });

  it('fails, without an output, when gh cannot publish', () => {
    const { run } = scenario({ drafts: [draft(TAG, ASSETS)], editFails: true });
    const result = run(publish);
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('gh release edit failed: HTTP 422');
    expect(result.output).toBe('');
  });

  it('needs a product and a tag, and knows two commands', () => {
    const { run } = scenario();
    expect(run(['publish', '--tag', TAG]).stderr).toContain('--product is required');
    expect(run(['bogus', '--product', 'cli', '--tag', TAG]).stderr).toContain(
      'Usage: release-publish.mjs publish|verify'
    );
  });
});

describe('release-publish.mjs verify', () => {
  const verify = (directory: string, ...extra: string[]) => [
    'verify',
    '--product',
    'cli',
    '--tag',
    TAG,
    '--directory',
    path.join(directory, 'published'),
    '--run-url',
    'https://github.com/wkh237/tmt/actions/runs/42',
    '--attempts',
    '1',
    ...extra,
  ];

  it('passes a public, immutable release whose attestation covers every asset, and opens no issue', () => {
    const fake = scenario();
    const result = fake.run(verify(fake.directory));
    expect(result.status).toBe(0);
    expect(result.summary).not.toContain('FAILED');
    expect(result.summary).toContain('- passed `attestation`: gh release verify passed');
    expect(result.summary).toContain(
      '- passed `assets`: gh release verify-asset passed for 3 assets'
    );
    expect(fake.calls().some(([, endpoint]) => endpoint.includes('/issues'))).toBe(false);
  });

  it('opens an issue and fails the run when an asset does not verify', () => {
    const fake = scenario({ badAssets: ['dist-manifest.json'] });
    const result = fake.run(verify(fake.directory));
    expect(result.status).toBe(1);
    expect(result.summary).toContain('- FAILED `assets`');
    expect(result.summary).toContain('dist-manifest.json: does not contain subject');
    expect(result.stderr).toContain('Opened issue #77.');
    const created = fake
      .calls()
      .find((call) => call[1] === 'repos/wkh237/tmt/issues' && call.includes('POST'));
    expect(created).toContain('title=Release v5.0.0-alpha.9 failed its post-publication checks');
    const body = created?.find((arg) => arg.startsWith('body='))?.slice(5);
    expect(body).toContain('dist-manifest.json: does not contain subject');
    expect(body).toContain('Run: https://github.com/wkh237/tmt/actions/runs/42');
    expect(body).toContain('Nothing was rolled back');
  });

  it('checks every published asset with gh release verify-asset', () => {
    const fake = scenario();
    expect(fake.run(verify(fake.directory)).status).toBe(0);
    const verified = fake
      .calls()
      .filter(([, sub]) => sub === 'verify-asset')
      .map((call) => path.basename(call[3] as string));
    expect(verified).toEqual(ASSETS);
    const download = fake.calls().find(([, sub]) => sub === 'download');
    expect(download).toEqual([
      'release',
      'download',
      TAG,
      '--repo',
      'wkh237/tmt',
      '--dir',
      path.join(fake.directory, 'published'),
    ]);
    expect(existsSync(path.join(fake.directory, 'published', 'dist-manifest.json'))).toBe(true);
  });

  it('comments on the issue that is already open instead of opening another', () => {
    const title = 'Release v5.0.0-alpha.9 failed its post-publication checks';
    const fake = scenario({ attestationFails: true, openIssues: [{ number: 12, title }] });
    const result = fake.run(verify(fake.directory));
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('Commented on issue #12.');
    const calls = fake.calls();
    expect(
      calls.some((call) => call[1] === 'repos/wkh237/tmt/issues' && call.includes('POST'))
    ).toBe(false);
    expect(calls.find((call) => call[1] === 'repos/wkh237/tmt/issues/12/comments')).toContain(
      'POST'
    );
  });

  it('fails the run for a release that is not immutable, a tag on another commit or a wrong latest', () => {
    for (const [name, options, expected] of [
      ['not immutable', { published: release({ immutable: false }) }, '`immutable`'],
      ['tag on another commit', { tagCommit: 'b'.repeat(40) }, '`tag`'],
      ['another latest', { latest: release({ tag_name: 'v5.0.0-alpha.8' }) }, '`flags`'],
    ] as const) {
      const fake = scenario(options);
      const result = fake.run(verify(fake.directory));
      expect(result.status, name).toBe(1);
      expect(result.summary, name).toContain(`- FAILED ${expected}`);
    }
  });

  it('still fails the run when the failure cannot be reported as an issue', () => {
    const fake = scenario({ attestationFails: true, issueFails: true });
    const result = fake.run(verify(fake.directory));
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('The failure could not be reported as an issue');
  });

  it('refuses a number of attempts that is not a positive whole number', () => {
    const fake = scenario();
    const arguments_ = verify(fake.directory);
    arguments_[arguments_.indexOf('--attempts') + 1] = '0';
    expect(fake.run(arguments_).stderr).toContain('--attempts must be a positive whole number');
  });
});

describe('release-publish.mjs report', () => {
  const report = (directory: string) => [
    'report',
    '--product',
    'cli',
    '--tag',
    TAG,
    '--directory',
    path.join(directory, 'smoke-failures'),
    '--run-url',
    'https://github.com/wkh237/tmt/actions/runs/42',
  ];
  const leave = (directory: string, target: string, content: string) => {
    const folder = path.join(directory, 'smoke-failures', `smoke-failures-cli-${TAG}-${target}`);
    mkdirSync(folder, { recursive: true });
    writeFileSync(path.join(folder, 'smoke-result.json'), content);
  };
  const issueBody = (calls: string[][]) => {
    const created = calls.find(
      (call) => call[1] === 'repos/wkh237/tmt/issues' && call.includes('POST')
    );
    return created?.find((arg) => arg.startsWith('body='))?.slice(5) ?? '';
  };

  it('opens the issue of the post-publication checks with each host’s failed checks', () => {
    const fake = scenario();
    leave(
      fake.directory,
      'aarch64-apple-darwin',
      JSON.stringify({
        target: 'aarch64-apple-darwin',
        failed: [{ check: 'tmt upgrade', reason: 'it reports 5.0.0-alpha.8, not 5.0.0-alpha.9' }],
      })
    );
    leave(
      fake.directory,
      'x86_64-unknown-linux-musl',
      JSON.stringify({
        target: 'x',
        failed: [{ check: 'install', reason: 'the installer exited 7' }],
      })
    );
    const result = fake.run(report(fake.directory));
    expect(result.status).toBe(0);
    expect(result.stderr).toContain('Opened issue #77.');
    expect(result.summary).toContain('- FAILED `install (x86_64-unknown-linux-musl)`');
    const calls = fake.calls();
    const created = calls.find(
      (call) => call[1] === 'repos/wkh237/tmt/issues' && call.includes('POST')
    );
    expect(created).toContain('title=Release v5.0.0-alpha.9 failed its post-publication checks');
    const body = issueBody(calls);
    expect(body).toContain(
      '- `tmt upgrade (aarch64-apple-darwin)`: it reports 5.0.0-alpha.8, not 5.0.0-alpha.9'
    );
    expect(body).toContain('- `install (x86_64-unknown-linux-musl)`: the installer exited 7');
    expect(body).toContain('Run: https://github.com/wkh237/tmt/actions/runs/42');
    expect(body).toContain('Nothing was rolled back');
    // It reports only: no release is read, edited or verified.
    expect(calls.filter(([command]) => command === 'release')).toEqual([]);
  });

  it('comments on the issue that is already open', () => {
    const title = 'Release v5.0.0-alpha.9 failed its post-publication checks';
    const fake = scenario({ openIssues: [{ number: 12, title }] });
    leave(
      fake.directory,
      'aarch64-apple-darwin',
      JSON.stringify({ failed: [{ check: 'a', reason: 'b' }] })
    );
    const result = fake.run(report(fake.directory));
    expect(result.stderr).toContain('Commented on issue #12.');
    expect(
      fake.calls().some((call) => call[1] === 'repos/wkh237/tmt/issues' && call.includes('POST'))
    ).toBe(false);
  });

  it('still reports a failure when no result file can be read, and keeps text to one line', () => {
    const none = scenario();
    expect(none.run(report(none.directory)).status).toBe(0);
    expect(issueBody(none.calls())).toContain(
      '- `public install`: a host failed and no details were kept; see the run'
    );
    const broken = scenario();
    leave(broken.directory, 'aarch64-apple-darwin', '{not json');
    broken.run(report(broken.directory));
    expect(issueBody(broken.calls())).toContain(
      '- `public install (aarch64-apple-darwin)`: its result file could not be read; see the run'
    );
    const unruly = scenario();
    leave(
      unruly.directory,
      'aarch64-apple-darwin',
      JSON.stringify({
        failed: [{ check: 'install', reason: `first\nsecond\u0007 ${'x'.repeat(900)}` }],
      })
    );
    unruly.run(report(unruly.directory));
    const reason =
      /`install \(aarch64-apple-darwin\)`: (.*)/.exec(issueBody(unruly.calls()))?.[1] ?? '';
    expect(reason.startsWith('first second')).toBe(true);
    expect(reason.length).toBeLessThanOrEqual(500);
  });

  it('fails the run when the issue cannot be opened, and needs a directory', () => {
    const fake = scenario({ issueFails: true });
    leave(
      fake.directory,
      'aarch64-apple-darwin',
      JSON.stringify({ failed: [{ check: 'a', reason: 'b' }] })
    );
    const result = fake.run(report(fake.directory));
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('The failure could not be reported as an issue');
    expect(fake.run(['report', '--product', 'cli', '--tag', TAG]).stderr).toContain(
      'report needs --directory'
    );
  });
});
