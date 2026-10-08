import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { createArtifact, nativeTarget } from '../support/native-artifact.js';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vite-plus/test';
import { colabFixtureBinary } from '../support/colab-runtime-fixture.js';
import { verifyColabApp } from '../../scripts/colab-runtime-proof.mjs';
import {
  installerUrl,
  LATEST_LAG_DEADLINE_MS,
  LATEST_LAG_POLL_MS,
  renderSmokeSummary,
  smokeRelease,
  type SmokeResult,
} from '../../scripts/verify-public-install.mjs';

let base: string;
beforeAll(() => {
  base = mkdtempSync(path.join(os.tmpdir(), 'public-install-'));
});
afterAll(() => rmSync(base, { recursive: true, force: true }));

const executableWriter = fileURLToPath(
  new URL('../support/executable-fixture.mjs', import.meta.url)
);
const shellQuote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
const { assertMacOsArchitecture, nativeHostTarget } = (await import(
  new URL('../../scripts/native-runtime-proof.mjs', import.meta.url).href
)) as {
  nativeHostTarget: () => string;
  assertMacOsArchitecture: (
    executable: string,
    target: string,
    options: { cwd: string; env: Record<string, string> },
    inspect: (command: string, args: string[]) => string
  ) => void;
};

const SKILL = '# The tmux-team skill\n';
const INBOX = '# The inbox skill\n';

interface Fake {
  driverExecutable?: string;
  tokenDigest?: string;
  persistCredential?: boolean;
  echoCredential?: boolean;
  /** The version the installer embeds and `tmt --version` prints. */
  version?: string;
  /** The version `tmt --version` prints, when it differs from the installer's. */
  installed?: string;
  skills?: Record<string, string>;
  /** Body of `tmt upgrade`'s JSON: the fields to override. */
  upgrade?: Record<string, unknown>;
  upgradeStderr?: string;
  upgradeCause?: string;
  upgradeFailures?: number;
  upgradeCode?: string;
  upgradeAfterStderr?: string;
  extensionCause?: string;
  extensionFailures?: number;
  upgradeStdout?: string;
  installerStatus?: number;
  withoutBinary?: boolean;
  /** An extension: whether the CLI has `tmt extension`, what it installs and lists, and whether it links the CLI. */
  extension?: {
    command?: boolean;
    installs?: string;
    reports?: string;
    link?: boolean;
    product?: string;
    binary?: string;
    notices?: string;
  };
}

/** The text of an installer that creates a fake `tmt` in the prefix, as the real one does. */
function installerText(fake: Fake = {}): string {
  const version = fake.version ?? '5.0.0-alpha.12';
  const skills = fake.skills ?? { 'tmux-team': SKILL, 'tmt-inbox': INBOX };
  const upgrade = {
    executable: '@EXE@',
    version,
    changed: false,
    channel: 'alpha',
    skills: { refreshed: [], skipped: [], conflicts: [] },
    pathWarning: null,
    ...fake.upgrade,
  };
  const extension = fake.extension ?? {};
  const requireToken = fake.tokenDigest
    ? `${shellQuote(globalThis.process.execPath)} -e 'if (require("node:crypto").createHash("sha256").update(process.env.GITHUB_TOKEN ?? "").digest("hex") !== "${fake.tokenDigest}" || process.env.GH_TOKEN) process.exit(1)' || exit 1`
    : 'true';
  const skillCommands = Object.entries(skills)
    .map(
      ([name, text]) =>
        `mkdir -p "$HOME/.agents/skills/${name}" && printf %s '${text}' > "$HOME/.agents/skills/${name}/SKILL.md"`
    )
    .join('\n');
  return `#!/bin/sh
set -eu
  version='${version}'
prefix=
${requireToken}
while [ "$#" -gt 0 ]; do case "$1" in --prefix) prefix=$2; shift 2 ;; *) shift ;; esac; done
[ "$(printf %s "\${CI:-}")" = true ]
if [ ${fake.installerStatus ?? 0} -ne 0 ]; then echo 'download failed' >&2; exit ${fake.installerStatus ?? 0}; fi
${fake.withoutBinary ? 'mkdir -p "$prefix/bin"' : ''}
${
  fake.withoutBinary
    ? ''
    : `mkdir -p "$prefix/bin"
${shellQuote(globalThis.process.execPath)} ${shellQuote(executableWriter)} --write "$prefix/bin/tmt" 493 <<'TMT'
#!/bin/sh
exe=$(cd "$(dirname "$0")" && pwd)/tmt
env | sed '/^GITHUB_TOKEN=/d' | sort > "$HOME/environment.txt"
case "$*" in
  --version) ${fake.tokenDigest ? '[ -z "${GITHUB_TOKEN:-}" ] || exit 1' : 'true'}; echo '${fake.installed ?? version}' ;;
  "upgrade --channel alpha --json")
    ${requireToken}
    ${fake.echoCredential ? 'printf %s "$GITHUB_TOKEN"; printf %s "$GITHUB_TOKEN" >&2; exit 1' : 'true'}
    ${fake.persistCredential ? 'printf %s "$GITHUB_TOKEN" > "$HOME/leaked-token"' : 'true'}
    count=$(cat "$HOME/upgrade-count" 2>/dev/null || echo 0); echo $((count + 1)) > "$HOME/upgrade-count"
    ${fake.upgradeCause ? `if [ "$count" -lt ${fake.upgradeFailures ?? 10} ]; then printf '%s' '${JSON.stringify({ error: { code: fake.upgradeCode ?? 'NATIVE_UPGRADE_FAILED', message: 'Native upgrade failed', cause: fake.upgradeCause } })}'; ${fake.upgradeStderr ? `echo '${fake.upgradeStderr}' >&2;` : ''} exit 1; fi` : ''}
    ${fake.upgradeAfterStderr ? `echo '${fake.upgradeAfterStderr}' >&2; exit 1` : ''}
    ${!fake.upgradeCause && fake.upgradeStderr ? `printf '%s' '${fake.upgradeStdout ?? ''}'; echo '${fake.upgradeStderr}' >&2; exit 1` : `printf '%s' '${JSON.stringify(upgrade)}' | sed "s#@EXE@#$exe#"`} ;;
  api) printf '{"dataRoot":"%s"}' "$TMUX_TEAM_HOME" ;;
  "extension install ${extension.product ?? 'ops'} "*)
    ${requireToken}
    count=$(cat "$HOME/extension-count" 2>/dev/null || echo 0); echo $((count + 1)) > "$HOME/extension-count"
    ${fake.extensionCause ? `if [ "$count" -lt ${fake.extensionFailures ?? 10} ]; then printf '%s' '${JSON.stringify({ error: { code: 'EXTENSION_INSTALL_FAILED', message: fake.extensionCause + ' Inspect with: tmt extension ls', cause: fake.extensionCause } })}'; exit 1; fi` : ''}
    ${extension.command === false ? 'echo "error: unrecognized subcommand \'extension\'" >&2; exit 2' : 'true'}
    prefix=$(echo "$*" | sed 's/.*--prefix //')
    mkdir -p "$prefix/lib" ${extension.link ? '"$prefix/bin" && : > "$prefix/bin/tmt"' : ''}
    ${
      extension.binary
        ? `mkdir -p "$prefix/lib/tmt-colab/releases/fixture" "$prefix/bin"
    cp ${shellQuote(extension.binary)} "$prefix/lib/tmt-colab/releases/fixture/tmt-colab"
    printf '%s' ${shellQuote(extension.notices ?? 'Rust attribution\nTiny app attribution\n')} > "$prefix/lib/tmt-colab/releases/fixture/THIRD-PARTY-NOTICES.txt"
    ln -s ../lib/tmt-colab/releases/fixture/tmt-colab "$prefix/bin/tmt-colab"`
        : ''
    }
    printf '{"extension":"${extension.product ?? 'ops'}","installed":true,"changed":true,"version":"%s"}' '${extension.installs ?? '0.1.0-alpha.4'}' ;;
  "extension list --json --prefix "*)
    printf '{"extensions":[{"name":"office","installed":false},{"name":"${extension.product ?? 'ops'}","installed":true,"version":"%s"}]}' '${extension.reports ?? extension.installs ?? '0.1.0-alpha.4'}' ;;
  "driver "*) ${fake.driverExecutable ? `exec ${shellQuote(fake.driverExecutable)} "$@"` : 'exit 9'} ;;
  *) echo "unexpected: $*" >&2; exit 9 ;;
esac
TMT
${skillCommands}`
}
`;
}

let counter = 0;
function run(
  fake: Fake,
  options: {
    product?: string;
    tag?: string;
    source?: Record<string, string>;
    fetches?: string[];
    systemPath?: string[];
    installerVersions?: string[];
    fetchError?: string;
    githubToken?: string;
    download?: (url: string, maximum: number) => Promise<Uint8Array>;
    target?: string;
    architectures?: string[];
    colabVariant?: string;
  } = {}
) {
  const root = path.join(base, `run-${(counter += 1)}`);
  mkdirSync(root);
  const source = path.join(root, 'source');
  for (const [name, text] of Object.entries(
    options.source ?? { 'tmux-team': SKILL, 'tmt-inbox': INBOX }
  )) {
    mkdirSync(path.join(source, 'skills', name), { recursive: true });
    writeFileSync(path.join(source, 'skills', name, 'SKILL.md'), text);
  }
  // A file that is not a skill directory, as the repository's own skills folder has.
  writeFileSync(path.join(source, 'skills', 'index.txt'), 'not a skill\n');
  const fetched: string[] = options.fetches ?? [];
  const waits: number[] = [];
  // A virtual clock that only `wait` advances, so the lag deadline is exact and instant.
  let clock = 0;
  const inspected: string[] = [];
  return {
    root,
    waits,
    fetched,
    inspected,
    results: smokeRelease({
      product: options.product ?? 'cli',
      tag: options.tag ?? 'v5.0.0-alpha.12',
      source,
      repository: 'wkh237/tmt',
      root: path.join(root, 'work'),
      target: options.target ?? nativeTarget(),
      inspectArchitecture: (executable, target, settings) => {
        assertMacOsArchitecture(executable, target, settings, (_command, args) => {
          if (args[0] === '--find') return '/selected/lipo';
          inspected.push(executable);
          return (
            options.architectures?.[inspected.length - 1] ??
            (globalThis.process.arch === 'arm64' ? 'arm64' : 'x86_64')
          );
        });
      },
      githubToken: options.githubToken,
      verifyColab: (input) =>
        verifyColabApp({ ...input, args: ['--fixture-variant', options.colabVariant ?? 'valid'] }),
      ...(options.download ? { download: options.download } : {}),
      ...(options.systemPath ? { systemPath: options.systemPath } : {}),
      fetch: async (url: string) => {
        fetched.push(url);
        if (options.fetchError) throw new Error(options.fetchError);
        return installerText({
          ...fake,
          ...(options.installerVersions
            ? {
                version:
                  options.installerVersions[
                    Math.min(fetched.length - 1, options.installerVersions.length - 1)
                  ],
              }
            : {}),
        });
      },
      wait: async (milliseconds: number) => {
        waits.push(milliseconds);
        clock += milliseconds;
      },
      now: () => clock,
    }),
  };
}

const failed = (results: SmokeResult[]) => results.filter(({ ok }) => !ok);

describe('the public installer smoke of a CLI release', () => {
  it('stops an arm64 install before running it and rejects an upgrade that switches architecture', async () => {
    const target = 'x86_64-apple-darwin';
    const control = run({}, { target, architectures: ['x86_64', 'x86_64'] });
    expect(failed(await control.results)).toEqual([]);
    expect(control.inspected).toHaveLength(2);
    const wrongInstall = run({}, { target, architectures: ['arm64'] });
    expect((await wrongInstall.results).at(-1)).toMatchObject({
      check: 'installed version',
      ok: false,
    });
    expect((await wrongInstall.results).at(-1)?.reason).toContain('exactly x86_64');
    expect(wrongInstall.inspected).toHaveLength(1);
    const wrongUpgrade = run({}, { target, architectures: ['x86_64', 'arm64'] });
    expect((await wrongUpgrade.results).at(-1)).toMatchObject({ check: 'tmt upgrade', ok: false });
    expect((await wrongUpgrade.results).at(-1)?.reason).toContain('exactly x86_64');
  });

  it('passes a release that installs, is selected by PATH, has its skills and is current', async () => {
    const attempt = run({});
    const results = await attempt.results;
    expect(results.map(({ check, ok }) => [check, ok])).toEqual([
      ['public installer', true],
      ['install', true],
      ['PATH selects the installed tmt', true],
      ['installed version', true],
      ['managed skills', true],
      ['tmt upgrade', true],
    ]);
    expect(results.at(-1)?.reason).toBe('5.0.0-alpha.12 is current');
    expect(results.find(({ check }) => check === 'managed skills')?.reason).toBe(
      'tmt-inbox, tmux-team'
    );
    expect(attempt.fetched).toEqual([installerUrl('wkh237/tmt')]);
    expect(installerUrl('wkh237/tmt')).toBe(
      'https://github.com/wkh237/tmt/releases/latest/download/install.sh'
    );
  });

  it.each(['cli', 'ops'])(
    'passes only the selected token to %s acquisition without persisting it',
    async (product) => {
      process.env.GH_TOKEN = 'secret-token';
      process.env.GITHUB_TOKEN = 'secret-token';
      try {
        const githubToken = 'fixture-acquisition-secret';
        const attempt = run(
          { tokenDigest: createHash('sha256').update(githubToken).digest('hex') },
          {
            githubToken,
            product,
            tag: product === 'cli' ? 'v5.0.0-alpha.12' : 'tmt-ops-v0.1.0-alpha.4',
          }
        );
        const results = await attempt.results;
        expect(failed(results)).toEqual([]);
        expect(
          JSON.stringify(results) +
            renderSmokeSummary({ tag: 'fixture', target: nativeTarget(), results })
        ).not.toContain(githubToken);
        const files = (directory: string): string[] =>
          readdirSync(directory, { withFileTypes: true }).flatMap((entry) =>
            entry.isDirectory()
              ? files(path.join(directory, entry.name))
              : entry.isFile()
                ? [path.join(directory, entry.name)]
                : []
          );
        for (const file of files(path.join(attempt.root, 'work')))
          expect(readFileSync(file).includes(githubToken), file).toBe(false);
        const environment = readFileSync(
          path.join(attempt.root, 'work', 'home', 'environment.txt'),
          'utf8'
        );
        expect(environment).toContain(`HOME=${path.join(attempt.root, 'work', 'home')}`);
        expect(environment).toContain(`TMUX_TEAM_HOME=${path.join(attempt.root, 'work', 'state')}`);
        expect(environment).toContain('CI=true');
        expect(environment).not.toMatch(/TOKEN|secret-token/);
        expect(environment).not.toContain(`HOME=${os.homedir()}`);
      } finally {
        delete process.env.GH_TOKEN;
        delete process.env.GITHUB_TOKEN;
      }
    }
  );

  it('passes with a note when a newer alpha appeared meanwhile, and fails for anything else', async () => {
    const newer = await run({ upgrade: { version: '5.0.0-alpha.13', changed: true } }).results;
    expect(failed(newer)).toEqual([]);
    expect(newer.at(-1)?.reason).toBe(
      'note: a newer alpha, 5.0.0-alpha.13, was published meanwhile and is installed'
    );
    for (const upgrade of [
      { version: '5.0.0-alpha.11', changed: true },
      { version: '5.0.0-alpha.12', changed: true },
      { version: '5.0.0', changed: true },
      { pathWarning: 'PATH selects another tmt' },
      { skills: { conflicts: ['a link'] } },
    ]) {
      const results = await run({ upgrade }).results;
      expect(results.at(-1), JSON.stringify(upgrade)).toMatchObject({
        check: 'tmt upgrade',
        ok: false,
      });
    }
    const elsewhere = await run({ upgrade: { executable: '/usr/bin/tmt' } }).results;
    expect(elsewhere.at(-1)?.reason).toContain('not');
  });

  it('fails latest-installer lag that outlasts the deadline with the unchanged message', async () => {
    const attempt = run({ version: '5.0.0-alpha.11' }, { tag: 'v5.0.0-alpha.12' });
    const results = await attempt.results;
    expect(results).toEqual([
      {
        check: 'public installer',
        ok: false,
        reason: 'the latest installer is for 5.0.0-alpha.11, not 5.0.0-alpha.12',
      },
    ]);
    const polls = LATEST_LAG_DEADLINE_MS / LATEST_LAG_POLL_MS;
    expect(attempt.waits).toEqual(Array(polls).fill(LATEST_LAG_POLL_MS));
    expect(attempt.fetched).toHaveLength(polls + 1);
  });

  it('recovers when latest catches up within the deadline and reports how long it lagged', async () => {
    const lagged = Array(8).fill('5.0.0-alpha.11');
    const attempt = run({}, { installerVersions: [...lagged, '5.0.0-alpha.12'] });
    const results = await attempt.results;
    expect(failed(results)).toEqual([]);
    expect(results[0]).toEqual({
      check: 'public installer',
      ok: true,
      reason: 'embeds 5.0.0-alpha.12 after latest lagged for 120s',
    });
    expect(attempt.fetched).toHaveLength(9);
    expect(attempt.waits).toEqual(Array(8).fill(LATEST_LAG_POLL_MS));
    expect((await run({}).results)[0]?.reason).toBe('embeds 5.0.0-alpha.12');
  });

  it('does not wait for malformed installer data or an unexpected newer version', async () => {
    const other = run({ version: 'invalid' });
    expect(failed(await other.results)).toHaveLength(1);
    expect(other.fetched).toHaveLength(1);
    expect(other.waits).toEqual([]);
    const newer = run({ version: '5.0.0' }, { tag: 'v5.0.0-alpha.12' });
    expect(failed(await newer.results)).toEqual([
      {
        check: 'public installer',
        ok: false,
        reason: 'the latest installer is for 5.0.0, not 5.0.0-alpha.12',
      },
    ]);
    expect(newer.fetched).toHaveLength(1);
    expect(newer.waits).toEqual([]);
  });

  it('proves an older independent CLI cut through its versioned installer while latest stays higher', async () => {
    const attempt = run({}, { installerVersions: ['5.0.0-alpha.13', '5.0.0-alpha.12'] });
    expect(failed(await attempt.results)).toEqual([]);
    expect(attempt.fetched).toEqual([
      installerUrl('wkh237/tmt'),
      installerUrl('wkh237/tmt', 'v5.0.0-alpha.12'),
    ]);
    expect(attempt.waits).toEqual([]);
  });
  it('does not retry an installer download error merely because latest reads allow lag recovery', async () => {
    const attempt = run({}, { fetchError: 'HTTP 403' });
    expect(failed(await attempt.results)).toEqual([
      { check: 'public installer', ok: false, reason: 'HTTP 403' },
    ]);
    expect(attempt.fetched).toHaveLength(1);
    expect(attempt.waits).toEqual([]);
  });

  it('fails an installer that exits nonzero, a missing tmt and a version that is not the installer’s', async () => {
    expect((await run({ installerStatus: 7 }).results).at(-1)?.reason).toContain(
      'the installer exited 7: download failed'
    );
    const missing = await run({ withoutBinary: true }).results;
    expect(missing.at(-1)).toMatchObject({ check: 'PATH selects the installed tmt', ok: false });
    expect(missing.at(-1)?.reason).toContain('PATH selects no tmt');
    // Another tmt that PATH reaches while the prefix has none is what PATH selects instead.
    const decoy = path.join(base, 'decoy');
    mkdirSync(decoy, { recursive: true });
    writeExecutable(path.join(decoy, 'tmt'), '#!/bin/sh\n', 0o644);
    const shadowing = run({ withoutBinary: true }, { systemPath: [decoy, '/usr/bin', '/bin'] });
    expect((await shadowing.results).at(-1)?.reason).toBe(
      `PATH selects ${path.join(decoy, 'tmt')}, not ${path.join(shadowing.root, 'work', 'prefix', 'bin', 'tmt')}`
    );
    const wrong = await run({ installed: '5.0.0-alpha.9' }).results;
    expect(wrong.at(-1)).toMatchObject({ check: 'installed version', ok: false });
    expect(wrong.at(-1)?.reason).toBe('tmt --version is 5.0.0-alpha.9, not 5.0.0-alpha.12');
  });

  it('compares the installed skills with the tag’s by name and by SKILL.md', async () => {
    const missing = await run({ skills: { 'tmux-team': SKILL } }).results;
    expect(missing.at(-1)).toMatchObject({ check: 'managed skills', ok: false });
    expect(missing.at(-1)?.reason).toBe(
      'installed skills are tmux-team, the release has tmt-inbox, tmux-team'
    );
    const different = await run({ skills: { 'tmux-team': '# other\n', 'tmt-inbox': INBOX } })
      .results;
    expect(different.at(-1)?.reason).toBe('tmux-team/SKILL.md differs from v5.0.0-alpha.12');
  });

  const diagnostic = (epoch = '1893456002') =>
    `GitHub API rate limit: reset/earliest retry time 2030-01-01 0:00:02.0 +00:00:00 (UTC epoch ${epoch}); the required wait exceeds the remaining deadline. Retry later or optionally set GITHUB_TOKEN.`;

  it('fails native acquisition rate limits immediately without a smoke retry', async () => {
    for (const product of ['cli', 'ops', 'driver-herdr']) {
      const attempt = run(
        { upgradeCause: diagnostic(), extensionCause: diagnostic() },
        {
          product,
          tag:
            product === 'cli'
              ? 'v5.0.0-alpha.12'
              : product === 'ops'
                ? 'tmt-ops-v0.1.0-alpha.4'
                : 'tmt-driver-herdr-v0.1.0-alpha.0',
        }
      );
      const failures = failed(await attempt.results);
      expect(failures).toHaveLength(1);
      expect(failures[0].reason).toContain('GitHub API rate limit');
      expect(failures[0]).not.toHaveProperty('infrastructure');
      expect(attempt.waits).toEqual([]);
      const count = product === 'ops' ? 'extension-count' : 'upgrade-count';
      expect(readFileSync(path.join(attempt.root, 'work/home', count), 'utf8').trim()).toBe('1');
    }
  });

  it('fails if an acquisition process persists the credential', async () => {
    const attempt = run({ persistCredential: true }, { githubToken: 'fixture-persisted-secret' });
    expect(failed(await attempt.results)).toEqual([
      {
        check: 'credential isolation',
        ok: false,
        reason: 'Acquisition credential persisted in installed state',
      },
    ]);
  });

  it('redacts a credential echoed in failed command diagnostics and summaries', async () => {
    const githubToken = 'fixture-diagnostic-secret';
    const attempt = run({ echoCredential: true }, { githubToken });
    const results = await attempt.results;
    expect(failed(results)).toHaveLength(1);
    const text =
      JSON.stringify(results) +
      renderSmokeSummary({ tag: 'v5.0.0-alpha.12', target: nativeTarget(), results });
    expect(text).not.toContain(githubToken);
    expect(text).toContain('[REDACTED]');
  });

  it.each([
    { upgradeStderr: 'HTTP 403: API rate limit exceeded' },
    { upgradeStderr: 'boom' },
    { upgradeCause: diagnostic(), upgradeStderr: 'an unrelated failure' },
    { upgradeCause: diagnostic(), upgradeCode: 'SOME_OTHER_FAILURE' },
    { upgradeCause: 'HTTP 429: too many requests' },
  ])('fails real or unclassified errors immediately: %j', async (fake) => {
    const attempt = run(fake);
    const result = (await attempt.results).at(-1);
    expect(result?.ok).toBe(false);
    expect(result).not.toHaveProperty('infrastructure');
    expect(attempt.waits).toEqual([]);
    expect(
      readFileSync(path.join(attempt.root, 'work', 'home', 'upgrade-count'), 'utf8').trim()
    ).toBe('1');
  });
});

describe('failed command diagnostics', () => {
  it.each([false, true, 'credential'])(
    'keeps failed CLI status and artifact evidence (rate limited: %s)',
    (limited) => {
      const root = path.join(base, `diagnostic-cli-${limited}`);
      const source = path.join(root, 'source');
      for (const [name, text] of Object.entries({ 'tmux-team': SKILL, 'tmt-inbox': INBOX })) {
        mkdirSync(path.join(source, 'skills', name), { recursive: true });
        writeFileSync(path.join(source, 'skills', name, 'SKILL.md'), text);
      }
      const preload = path.join(root, 'fetch.mjs');
      writeExecutable(
        preload,
        `import { createRequire, syncBuiltinESMExports } from 'node:module';
const child = createRequire(import.meta.url)('node:child_process');
const spawn = child.spawnSync;
child.spawnSync = (command, ...args) => command.endsWith('/lipo')
  ? { status: 0, signal: null, stdout: '${globalThis.process.arch === 'arm64' ? 'arm64' : 'x86_64'}', stderr: '' }
  : spawn(command, ...args);
syncBuiltinESMExports();
globalThis.fetch = async () => ({ ok: true, text: async () => ${JSON.stringify(
          installerText(
            limited === true
              ? {
                  upgradeCause:
                    'GitHub API rate limit: reset/earliest retry time 2030-01-01 (UTC epoch 1893456000); the required wait exceeds the remaining deadline. Retry later or optionally set GITHUB_TOKEN.',
                }
              : limited === 'credential'
                ? { echoCredential: true }
                : {
                    upgradeStdout: 'stdout-cause-' + 'x'.repeat(4000),
                    upgradeStderr: 'stderr-cause-' + 'y'.repeat(4000),
                  }
          )
        )} });\nglobalThis.setTimeout = (fn) => { fn(); return 0; };\n`,
        0o644
      );
      // No real network or retry sleeps: timers are immediate only in this isolated test process.
      const resultFile = path.join(root, 'result.json');
      const process = spawnSync(
        globalThis.process.execPath,
        [
          '--import',
          preload,
          fileURLToPath(new URL('../../scripts/verify-public-install.mjs', import.meta.url)),
          '--product',
          'cli',
          '--tag',
          'v5.0.0-alpha.12',
          '--source',
          source,
          '--target',
          nativeHostTarget(),
          '--result-file',
          resultFile,
        ],
        {
          encoding: 'utf8',
          timeout: 20_000,
          env: {
            ...globalThis.process.env,
            GITHUB_REPOSITORY: 'pj-tmt/tmt',
            GITHUB_TOKEN: 'fixture-process-secret',
          },
        }
      );
      expect(process.error).toBeUndefined();
      expect(process.status).toBe(1);
      const result = JSON.parse(readFileSync(resultFile, 'utf8'));
      expect(result.target).toBe(nativeHostTarget());
      expect(result.failed).toHaveLength(1);
      const failure = result.failed[0];
      expect(failure.check).toBe('tmt upgrade');
      expect(failure.reason.length).toBeLessThanOrEqual(500);
      for (const text of [process.stdout, process.stderr, readFileSync(resultFile, 'utf8')])
        expect(text).not.toContain('fixture-process-secret');
      if (limited === 'credential') {
        expect(failure.reason).toContain('[REDACTED]');
        return;
      }
      if (limited === true) {
        expect(failure).not.toHaveProperty('infrastructure');
        expect(failure.detail).toContain('GitHub API rate limit');
        expect(process.stderr).toContain('GitHub API rate limit');
        return;
      }
      expect(failure).not.toHaveProperty('infrastructure');
      expect(failure.detail.length).toBeLessThanOrEqual(6000);
      for (const text of [
        'Packed command failed (exited 1, expected 0)',
        'stdout: stdout-cause-',
        'stderr: stderr-cause-',
        '(4013 characters)',
      ]) {
        expect(failure.detail).toContain(text);
        expect(process.stderr).toContain(text);
      }
      expect(failure.detail).not.toContain('x'.repeat(2001));
      expect(failure.detail).not.toContain('y'.repeat(2001));
    }
  );
});

describe('the public installer smoke of an extension release', () => {
  const tag = 'tmt-ops-v0.1.0-alpha.4';
  const smoke = (extension: Fake['extension'] = {}) =>
    run({ extension }, { product: 'ops', tag }).results;

  it('installs the newest CLI, then the extension, and checks its version and that no CLI link appears', async () => {
    const results = await smoke();
    expect(results.map(({ check, ok }) => [check, ok])).toEqual([
      ['public installer', true],
      ['install', true],
      ['PATH selects the installed tmt', true],
      ['installed version', true],
      ['ops install', true],
      ['ops list', true],
    ]);
    // The installer's version is the CLI's, not the extension tag's, and nothing checks the skills.
    expect(results.find(({ check }) => check === 'installed version')?.reason).toBe(
      '5.0.0-alpha.12'
    );
  });

  it('names an install through a CLI without `tmt extension`, and never uses a command of the extension itself', async () => {
    const results = await smoke({ command: false });
    expect(results.at(-1)).toMatchObject({ check: 'ops install', ok: false });
    expect(results.at(-1)?.reason).toContain("unrecognized subcommand 'extension'");
    // The fake knows no `tmt ops ...`: a verifier that used one would fail the passing case.
    expect((await smoke()).every(({ ok }) => ok)).toBe(true);
  });

  it('fails an install of another version, a list that disagrees and a CLI link', async () => {
    expect((await smoke({ installs: '0.1.0-alpha.3' })).at(-1)?.reason).toBe(
      'it installed 0.1.0-alpha.3, not 0.1.0-alpha.4'
    );
    expect((await smoke({ reports: '0.1.0-alpha.3' })).at(-1)?.reason).toBe(
      'the list reports 0.1.0-alpha.3, not 0.1.0-alpha.4'
    );
    const linked = await smoke({ link: true });
    expect(linked.at(-1)).toMatchObject({ check: 'ops list', ok: false });
    expect(linked.at(-1)?.reason).toContain('must not create the CLI link');
  });

  it('inspects both the installed CLI driver and extension bytes for an Intel target', async () => {
    const options = { product: 'ops', tag, target: 'x86_64-apple-darwin' };
    const control = run({}, { ...options, architectures: ['x86_64', 'x86_64'] });
    expect(failed(await control.results)).toEqual([]);
    expect(control.inspected.map((file) => path.basename(file))).toEqual(['tmt', 'tmt-ops']);
    const wrong = run({}, { ...options, architectures: ['x86_64', 'arm64'] });
    expect((await wrong.results).at(-1)).toMatchObject({ check: 'ops install', ok: false });
    expect((await wrong.results).at(-1)?.reason).toContain('exactly x86_64');
  });
});

describe('public Colab embedded app smoke', () => {
  it('keeps authenticated acquisition separate from the relocated app process', async () => {
    const githubToken = 'fixture-colab-acquisition-secret';
    const attempt = run(
      {
        tokenDigest: createHash('sha256').update(githubToken).digest('hex'),
        extension: {
          product: 'colab',
          binary: colabFixtureBinary(base),
          installs: '0.1.0-alpha.1',
        },
      },
      { product: 'colab', tag: 'tmt-colab-v0.1.0-alpha.1', githubToken }
    );
    const results = await attempt.results;
    // Acquisition requires the credential; the native app fixture refuses it at runtime.
    expect(failed(results)).toEqual([]);
    expect(results.at(-1)).toMatchObject({ check: 'colab embedded app', ok: true });
    expect(JSON.stringify(results)).not.toContain(githubToken);
  });

  it.each([
    ['valid', true],
    ['PLACEHOLDER', false],
    ['STARTUP_FAILURE', false],
    ['LEAK_SOCKET', false],
  ])('checks an installed native %s fixture after install/list', async (variant, ok) => {
    const binary = colabFixtureBinary(base);
    const attempt = run(
      { extension: { product: 'colab', binary, installs: '0.1.0-alpha.1' } },
      { product: 'colab', tag: 'tmt-colab-v0.1.0-alpha.1', colabVariant: variant }
    );
    const results = await attempt.results;
    expect(results.slice(0, -1).every((result) => result.ok)).toBe(true);
    expect(results.at(-1)).toMatchObject({ check: 'colab embedded app', ok });
    if (!ok)
      expect(results.at(-1)?.reason).toMatch(
        /placeholder|COLAB_APP_UNAVAILABLE|clean up its socket/
      );
  });

  it('rejects an installed artifact whose notices omit the frontend', async () => {
    const attempt = run(
      {
        extension: {
          product: 'colab',
          binary: colabFixtureBinary(base),
          installs: '0.1.0-alpha.1',
          notices: 'Rust attribution\n',
        },
      },
      { product: 'colab', tag: 'tmt-colab-v0.1.0-alpha.1' }
    );
    expect((await attempt.results).at(-1)).toMatchObject({
      check: 'colab embedded app',
      ok: false,
    });
    expect((await attempt.results).at(-1)?.reason).toContain(
      'combined notices omit Rust or frontend'
    );
  });
});

describe('renderSmokeSummary', () => {
  it('lists each check with its reason', () => {
    expect(
      renderSmokeSummary({
        tag: 'v5.0.0-alpha.12',
        target: 'aarch64-apple-darwin',
        results: [
          { check: 'install', ok: true, reason: '' },
          { check: 'tmt upgrade', ok: false, reason: 'it reports 5.0.0' },
        ],
      })
    ).toBe(
      '### Public install of `v5.0.0-alpha.12` (aarch64-apple-darwin)\n\n- passed install\n- FAILED tmt upgrade: it reports 5.0.0\n'
    );
  });
});

describe('public standalone driver smoke', () => {
  it('inspects the upgraded public CLI and driver archive before Intel approval', async () => {
    const target = 'x86_64-apple-darwin';
    const fixtureRoot = mkdtempSync(path.join(base, 'driver-intel-oracle-'));
    const archiveName = `tmt-driver-herdr-${target}.tar.gz`;
    const artifact = await createArtifact(
      { root: fixtureRoot },
      '0.1.0-alpha.0',
      new Uint8Array(),
      'driver-herdr',
      undefined,
      {},
      archiveName
    );
    // This orchestration fixture's byte oracle supplies architecture; real lipo has its own tests.
    const manifest = JSON.parse(readFileSync(artifact.manifest, 'utf8'));
    manifest.artifacts[archiveName].target_triples = [target];
    writeFileSync(artifact.manifest, JSON.stringify(manifest));
    const smoke = (architectures: string[]) =>
      run(
        {
          driverExecutable: path.resolve('../rust/target/debug/tmt'),
        },
        {
          product: 'driver-herdr',
          tag: 'tmt-driver-herdr-v0.1.0-alpha.0',
          target,
          architectures,
          download: async (url) =>
            readFileSync(
              url.endsWith('/dist-manifest.json') ? artifact.manifest : artifact.archive
            ),
        }
      );
    const control = smoke(['x86_64', 'x86_64', 'x86_64']);
    expect(failed(await control.results)).toEqual([]);
    expect(control.inspected.map((file) => path.basename(file))).toEqual([
      'tmt',
      'tmt',
      'tmt-driver-herdr',
    ]);
    for (const [architectures, check] of [
      [['x86_64', 'arm64'], 'current public CLI'],
      [['x86_64', 'x86_64', 'arm64'], 'driver public archive and approval'],
    ] as const) {
      const wrong = smoke([...architectures]);
      expect((await wrong.results).at(-1)).toMatchObject({ check, ok: false });
      expect((await wrong.results).at(-1)?.reason).toContain('exactly x86_64');
    }
  });

  it('verifies the public archive and durable approval through the current CLI', async () => {
    const fixtureRoot = mkdtempSync(path.join(base, 'driver-archive-'));
    const archiveName = `tmt-driver-herdr-${nativeTarget()}.tar.gz`;
    const artifact = await createArtifact(
      { root: fixtureRoot },
      '0.1.0-alpha.0',
      new Uint8Array(),
      'driver-herdr',
      undefined,
      {},
      archiveName
    );
    const urls: string[] = [];
    const fake = {
      driverExecutable: path.resolve('../rust/target/debug/tmt'),
    };
    const attempt = run(fake, {
      product: 'driver-herdr',
      tag: 'tmt-driver-herdr-v0.1.0-alpha.0',
      download: async (url, maximum) => {
        urls.push(url);
        const bytes = readFileSync(
          url.endsWith('/dist-manifest.json') ? artifact.manifest : artifact.archive
        );
        expect(bytes.length).toBeLessThan(maximum);
        return bytes;
      },
    });
    expect(failed(await attempt.results)).toEqual([]);
    expect(attempt.waits).toEqual([]);
    expect(urls).toEqual(
      ['dist-manifest.json', archiveName].map(
        (name) =>
          `https://github.com/wkh237/tmt/releases/download/tmt-driver-herdr-v0.1.0-alpha.0/${name}`
      )
    );
    const registry = JSON.parse(
      readFileSync(path.join(attempt.root, 'work/state/drivers.json'), 'utf8')
    );
    expect(JSON.stringify(registry)).toContain('0.1.0-alpha.0');
  });

  it('fails immediately on an unclassified public archive HTTP failure', async () => {
    let downloads = 0;
    const attempt = run(
      {},
      {
        product: 'driver-herdr',
        tag: 'tmt-driver-herdr-v0.1.0-alpha.0',
        download: async () => {
          downloads++;
          throw new Error('HTTP 403');
        },
      }
    );
    expect(failed(await attempt.results)[0].check).toBe('driver public archive and approval');
    expect(downloads).toBe(1);
    expect(attempt.waits).toEqual([]);
  });
});
