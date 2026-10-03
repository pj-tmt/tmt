import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { createArtifact, nativeTarget } from '../support/native-artifact.js';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vite-plus/test';
import {
  installerUrl,
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
  extension?: { command?: boolean; installs?: string; reports?: string; link?: boolean };
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
env | sort > "$HOME/environment.txt"
case "$*" in
  --version) echo '${fake.installed ?? version}' ;;
  "upgrade --channel alpha --json")
    count=$(cat "$HOME/upgrade-count" 2>/dev/null || echo 0); echo $((count + 1)) > "$HOME/upgrade-count"
    ${fake.upgradeCause ? `if [ "$count" -lt ${fake.upgradeFailures ?? 10} ]; then printf '%s' '${JSON.stringify({ error: { code: fake.upgradeCode ?? 'NATIVE_UPGRADE_FAILED', message: 'Native upgrade failed', cause: fake.upgradeCause } })}'; ${fake.upgradeStderr ? `echo '${fake.upgradeStderr}' >&2;` : ''} exit 1; fi` : ''}
    ${fake.upgradeAfterStderr ? `echo '${fake.upgradeAfterStderr}' >&2; exit 1` : ''}
    ${!fake.upgradeCause && fake.upgradeStderr ? `printf '%s' '${fake.upgradeStdout ?? ''}'; echo '${fake.upgradeStderr}' >&2; exit 1` : `printf '%s' '${JSON.stringify(upgrade)}' | sed "s#@EXE@#$exe#"`} ;;
  "extension install squad "*)
    count=$(cat "$HOME/extension-count" 2>/dev/null || echo 0); echo $((count + 1)) > "$HOME/extension-count"
    ${fake.extensionCause ? `if [ "$count" -lt ${fake.extensionFailures ?? 10} ]; then printf '%s' '${JSON.stringify({ error: { code: 'EXTENSION_INSTALL_FAILED', message: fake.extensionCause + ' Inspect with: tmt extension ls', cause: fake.extensionCause } })}'; exit 1; fi` : ''}
    ${extension.command === false ? 'echo "error: unrecognized subcommand \'extension\'" >&2; exit 2' : 'true'}
    prefix=$(echo "$*" | sed 's/.*--prefix //')
    mkdir -p "$prefix/lib" ${extension.link ? '"$prefix/bin" && : > "$prefix/bin/tmt"' : ''}
    printf '{"extension":"squad","installed":true,"changed":true,"version":"%s"}' '${extension.installs ?? '0.1.0-alpha.4'}' ;;
  "extension list --json --prefix "*)
    printf '{"extensions":[{"name":"office","installed":false},{"name":"squad","installed":true,"version":"%s"}]}' '${extension.reports ?? extension.installs ?? '0.1.0-alpha.4'}' ;;
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
    retry?: boolean;
    download?: (url: string, maximum: number) => Promise<Uint8Array>;
    target?: string;
    architectures?: string[];
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
      now: () => 1893456000000,
      retry: options.retry,
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
      },
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

  it('runs everything in an isolated environment that carries no token', async () => {
    process.env.GH_TOKEN = 'secret-token';
    process.env.GITHUB_TOKEN = 'secret-token';
    try {
      const attempt = run({});
      await attempt.results;
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
  });

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

  it('bounds the specifically classified previous-version latest-installer lag', async () => {
    const attempt = run({ version: '5.0.0-alpha.11' }, { tag: 'v5.0.0-alpha.12' });
    const results = await attempt.results;
    expect(results).toEqual([
      {
        check: 'public installer',
        ok: false,
        reason: 'the latest installer is for 5.0.0-alpha.11, not 5.0.0-alpha.12',
      },
    ]);
    expect(attempt.fetched).toHaveLength(3);
    expect(attempt.waits).toEqual([20_000, 20_000]);
  });

  it('recovers when latest catches up, without retrying a newer or malformed installer', async () => {
    const attempt = run({}, { installerVersions: ['5.0.0-alpha.11', '5.0.0-alpha.12'] });
    expect(failed(await attempt.results)).toEqual([]);
    expect(attempt.fetched).toHaveLength(2);
    expect(attempt.waits).toEqual([20_000]);
    for (const version of ['5.0.0-alpha.13', 'invalid']) {
      const other = run({ version });
      expect(failed(await other.results)).toHaveLength(1);
      expect(other.fetched).toHaveLength(1);
      expect(other.waits).toEqual([]);
    }
  });

  it('does not retry an installer download error merely because latest reads allow lag recovery', async () => {
    const attempt = run({}, { fetchError: 'HTTP 403' });
    expect(failed(await attempt.results)).toEqual([
      { check: 'public installer', ok: false, reason: 'HTTP 403' },
    ]);
    expect(attempt.fetched).toHaveLength(1);
    expect(attempt.waits).toEqual([]);
  });

  it('pins the native diagnostic format, timing representation and reasons consumed by smoke', () => {
    const rust = readFileSync(
      new URL('../../../rust/crates/tmt-adapters/src/release_http.rs', import.meta.url),
      'utf8'
    );
    expect(rust.match(/"GitHub API rate limit:[^"\n]+"/)?.[0]).toBe(
      '"GitHub API rate limit: reset/earliest retry time {reset}; {reason}. Retry later or optionally set GITHUB_TOKEN."'
    );
    expect(rust).toContain('format!("{date} (UTC epoch {epoch})")');
    expect(rust).toContain('Some("the single retry was exhausted")');
    expect(rust).toContain('Some("the required wait exceeds the remaining deadline")');
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

  it('retries only the upgrade after its reset and preserves the already completed install', async () => {
    const attempt = run({ upgradeCause: diagnostic(), upgradeFailures: 1 });
    expect(failed(await attempt.results)).toEqual([]);
    expect(attempt.waits).toEqual([3000]);
    expect(attempt.fetched).toHaveLength(1);
    expect(
      readFileSync(path.join(attempt.root, 'work', 'home', 'upgrade-count'), 'utf8').trim()
    ).toBe('2');
  });

  it('fails a real error after a rate-limit retry without retaining infrastructure classification', async () => {
    const attempt = run({
      upgradeCause: diagnostic(),
      upgradeFailures: 1,
      upgradeAfterStderr: 'archive corrupt',
    });
    const result = (await attempt.results).at(-1);
    expect(result?.ok).toBe(false);
    expect(result?.reason).toContain('archive corrupt');
    expect(result?.infrastructure).toBeUndefined();
    expect(attempt.waits).toEqual([3000]);
  });

  it('retries extension acquisition alone using its native error cause', async () => {
    const attempt = run(
      { extensionCause: diagnostic(), extensionFailures: 1 },
      { product: 'squad', tag: 'tmt-squad-v0.1.0-alpha.4' }
    );
    expect(failed(await attempt.results)).toEqual([]);
    expect(attempt.waits).toEqual([3000]);
    expect(attempt.fetched).toHaveLength(1);
    expect(
      readFileSync(path.join(attempt.root, 'work', 'home', 'extension-count'), 'utf8').trim()
    ).toBe('2');
  });

  it('keeps a repeated rate limit as a failed infrastructure conclusion after two attempts', async () => {
    const attempt = run({ upgradeCause: diagnostic() });
    expect((await attempt.results).at(-1)).toMatchObject({
      ok: false,
      infrastructure: 'github-api-rate-limit',
    });
    expect((await attempt.results).at(-1)?.reason).toContain('attempt bound exceeded (2 attempts)');
    expect(attempt.waits).toEqual([3000]);
    expect(
      readFileSync(path.join(attempt.root, 'work', 'home', 'upgrade-count'), 'utf8').trim()
    ).toBe('2');
  });

  it('preserves the exact reset diagnostic and allows no further acquisition attempt in a deferred retry', async () => {
    const attempt = run({ upgradeCause: diagnostic(), upgradeFailures: 1 }, { retry: true });
    expect((await attempt.results).at(-1)).toMatchObject({
      ok: false,
      infrastructure: 'github-api-rate-limit',
      rateLimit: { diagnostic: diagnostic(), resetAtMs: 1893456002000 },
    });
    expect(attempt.waits).toEqual([]);
    expect(
      readFileSync(path.join(attempt.root, 'work', 'home', 'upgrade-count'), 'utf8').trim()
    ).toBe('1');
    expect(failed(await run({}, { retry: true }).results)).toEqual([]);
  });

  it.each([
    diagnostic('1893456600'),
    diagnostic().replace(/2030[^;]+/, 'unavailable (missing or invalid timing header)'),
  ])('fails clearly without waiting beyond the bound or guessing missing timing', async (cause) => {
    const attempt = run({ upgradeCause: cause });
    const result = (await attempt.results).at(-1);
    expect(result?.infrastructure).toBe('github-api-rate-limit');
    expect(result?.reason).toMatch(/wait bound exceeded|reset time unavailable/);
    expect(attempt.waits).toEqual([]);
    expect(
      readFileSync(path.join(attempt.root, 'work', 'home', 'upgrade-count'), 'utf8').trim()
    ).toBe('1');
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
    expect(result?.infrastructure).toBeUndefined();
    expect(attempt.waits).toEqual([]);
    expect(
      readFileSync(path.join(attempt.root, 'work', 'home', 'upgrade-count'), 'utf8').trim()
    ).toBe('1');
  });
});

describe('failed command diagnostics', () => {
  it.each([false, true])(
    'keeps failed CLI status and artifact evidence (infrastructure: %s)',
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
            limited
              ? {
                  upgradeCause:
                    'GitHub API rate limit: reset/earliest retry time 2030-01-01 (UTC epoch 1893456000); the required wait exceeds the remaining deadline. Retry later or optionally set GITHUB_TOKEN.',
                }
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
          env: { ...globalThis.process.env, GITHUB_REPOSITORY: 'pj-tmt/tmt' },
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
      if (limited) {
        expect(failure.infrastructure).toBe('github-api-rate-limit');
        expect(failure.reason).toContain('wait bound exceeded');
        expect(process.stderr).toContain('Public install infrastructure');
        return;
      }
      expect(failure.infrastructure).toBeUndefined();
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
  const tag = 'tmt-squad-v0.1.0-alpha.4';
  const smoke = (extension: Fake['extension'] = {}) =>
    run({ extension }, { product: 'squad', tag }).results;

  it('installs the newest CLI, then the extension, and checks its version and that no CLI link appears', async () => {
    const results = await smoke();
    expect(results.map(({ check, ok }) => [check, ok])).toEqual([
      ['public installer', true],
      ['install', true],
      ['PATH selects the installed tmt', true],
      ['installed version', true],
      ['squad install', true],
      ['squad list', true],
    ]);
    // The installer's version is the CLI's, not the extension tag's, and nothing checks the skills.
    expect(results.find(({ check }) => check === 'installed version')?.reason).toBe(
      '5.0.0-alpha.12'
    );
  });

  it('names an install through a CLI without `tmt extension`, and never uses a command of the extension itself', async () => {
    const results = await smoke({ command: false });
    expect(results.at(-1)).toMatchObject({ check: 'squad install', ok: false });
    expect(results.at(-1)?.reason).toContain("unrecognized subcommand 'extension'");
    // The fake knows no `tmt squad ...`: a verifier that used one would fail the passing case.
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
    expect(linked.at(-1)).toMatchObject({ check: 'squad list', ok: false });
    expect(linked.at(-1)?.reason).toContain('must not create the CLI link');
  });

  it('inspects both the installed CLI driver and extension bytes for an Intel target', async () => {
    const options = { product: 'squad', tag, target: 'x86_64-apple-darwin' };
    const control = run({}, { ...options, architectures: ['x86_64', 'x86_64'] });
    expect(failed(await control.results)).toEqual([]);
    expect(control.inspected.map((file) => path.basename(file))).toEqual(['tmt', 'tmt-squad']);
    const wrong = run({}, { ...options, architectures: ['x86_64', 'arm64'] });
    expect((await wrong.results).at(-1)).toMatchObject({ check: 'squad install', ok: false });
    expect((await wrong.results).at(-1)?.reason).toContain('exactly x86_64');
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

  it('verifies the public archive and durable approval through the current CLI, reusing classified retry', async () => {
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
      upgradeCause:
        'GitHub API rate limit: reset/earliest retry time 2030-01-01T00:00:01Z (UTC epoch 1893456001); the single retry was exhausted. Retry later or optionally set GITHUB_TOKEN.',
      upgradeFailures: 1,
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
    expect(attempt.waits).toEqual([2000]);
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

  it('uses one CLI acquisition attempt for a deferred driver re-proof', async () => {
    let downloads = 0;
    const attempt = run(
      {
        upgradeFailures: 1,
        upgradeCause:
          'GitHub API rate limit: reset/earliest retry time 2030-01-01T00:00:01Z (UTC epoch 1893456001); the single retry was exhausted. Retry later or optionally set GITHUB_TOKEN.',
      },
      {
        product: 'driver-herdr',
        tag: 'tmt-driver-herdr-v0.1.0-alpha.0',
        retry: true,
        download: async () => {
          downloads++;
          throw new Error('unexpected archive acquisition');
        },
      }
    );
    expect(failed(await attempt.results)).toEqual([
      expect.objectContaining({
        check: 'current public CLI',
        infrastructure: 'github-api-rate-limit',
        reason: expect.stringContaining('attempt bound exceeded (1 attempts)'),
      }),
    ]);
    expect(attempt.waits).toEqual([]);
    expect(downloads).toBe(0);
    expect(
      readFileSync(path.join(attempt.root, 'work', 'home', 'upgrade-count'), 'utf8').trim()
    ).toBe('1');
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
