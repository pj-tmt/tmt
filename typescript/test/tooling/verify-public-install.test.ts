import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
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

const SKILL = '# The tmux-team skill\n';
const INBOX = '# The inbox skill\n';

interface Fake {
  /** The version the installer embeds and `tmt --version` prints. */
  version?: string;
  /** The version `tmt --version` prints, when it differs from the installer's. */
  installed?: string;
  skills?: Record<string, string>;
  /** Body of `tmt upgrade`'s JSON: the fields to override. */
  upgrade?: Record<string, unknown>;
  upgradeStderr?: string;
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
cat > "$prefix/bin/tmt" <<'TMT'
#!/bin/sh
exe=$(cd "$(dirname "$0")" && pwd)/tmt
env | sort > "$HOME/environment.txt"
case "$*" in
  --version) echo '${fake.installed ?? version}' ;;
  "upgrade --channel alpha --json")
    count=$(cat "$HOME/upgrade-count" 2>/dev/null || echo 0); echo $((count + 1)) > "$HOME/upgrade-count"
    ${fake.upgradeStderr ? `printf '%s' '${fake.upgradeStdout ?? ''}'; echo '${fake.upgradeStderr}' >&2; exit 1` : `printf '%s' '${JSON.stringify(upgrade)}' | sed "s#@EXE@#$exe#"`} ;;
  "extension install squad "*)
    ${extension.command === false ? 'echo "error: unrecognized subcommand \'extension\'" >&2; exit 2' : 'true'}
    prefix=$(echo "$*" | sed 's/.*--prefix //')
    mkdir -p "$prefix/lib" ${extension.link ? '"$prefix/bin" && : > "$prefix/bin/tmt"' : ''}
    printf '{"extension":"squad","installed":true,"changed":true,"version":"%s"}' '${extension.installs ?? '0.1.0-alpha.4'}' ;;
  "extension list --json --prefix "*)
    printf '{"extensions":[{"name":"office","installed":false},{"name":"squad","installed":true,"version":"%s"}]}' '${extension.reports ?? extension.installs ?? '0.1.0-alpha.4'}' ;;
  *) echo "unexpected: $*" >&2; exit 9 ;;
esac
TMT
chmod 755 "$prefix/bin/tmt"
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
  return {
    root,
    waits,
    fetched,
    results: smokeRelease({
      product: options.product ?? 'cli',
      tag: options.tag ?? 'v5.0.0-alpha.12',
      source,
      repository: 'wkh237/tmt',
      root: path.join(root, 'work'),
      ...(options.systemPath ? { systemPath: options.systemPath } : {}),
      fetch: async (url: string) => {
        fetched.push(url);
        return installerText(fake);
      },
      wait: async (milliseconds: number) => {
        waits.push(milliseconds);
      },
    }),
  };
}

const failed = (results: SmokeResult[]) => results.filter(({ ok }) => !ok);

describe('the public installer smoke of a CLI release', () => {
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

  it('retries a stale latest installer, then fails naming both versions', async () => {
    const attempt = run({ version: '5.0.0-alpha.11' }, { tag: 'v5.0.0-alpha.12' });
    const results = await attempt.results;
    expect(results).toEqual([
      {
        check: 'public installer',
        ok: false,
        reason:
          'the public installer failed after 3 attempts: the latest installer is for 5.0.0-alpha.11, not 5.0.0-alpha.12',
      },
    ]);
    expect(attempt.fetched).toHaveLength(3);
    expect(attempt.waits).toEqual([20_000, 20_000]);
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
    writeFileSync(path.join(decoy, 'tmt'), '#!/bin/sh\n');
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

  it('retries the upgrade, reports a rate limit as one, and does not retry a wrong answer', async () => {
    const attempt = run({ upgradeStderr: 'HTTP 403: API rate limit exceeded' });
    const results = await attempt.results;
    expect(results.at(-1)?.ok).toBe(false);
    expect(results.at(-1)?.reason).toMatch(
      /^GitHub rate limit: tmt upgrade failed after 3 attempts/
    );
    expect(
      readFileSync(path.join(attempt.root, 'work', 'home', 'upgrade-count'), 'utf8').trim()
    ).toBe('3');
    expect(attempt.waits).toEqual([20_000, 20_000]);
    const other = await run({ upgradeStderr: 'boom' }).results;
    expect(other.at(-1)?.reason).not.toContain('rate limit');
  });
});

describe('failed command diagnostics', () => {
  it('keeps both bounded streams in the run log and result file after retries', () => {
    const root = path.join(base, 'diagnostic-cli');
    const source = path.join(root, 'source');
    for (const [name, text] of Object.entries({ 'tmux-team': SKILL, 'tmt-inbox': INBOX })) {
      mkdirSync(path.join(source, 'skills', name), { recursive: true });
      writeFileSync(path.join(source, 'skills', name, 'SKILL.md'), text);
    }
    const preload = path.join(root, 'fetch.mjs');
    writeFileSync(
      preload,
      `globalThis.fetch = async () => ({ ok: true, text: async () => ${JSON.stringify(
        installerText({
          upgradeStdout: 'stdout-cause-' + 'x'.repeat(4000),
          upgradeStderr: 'stderr-cause-' + 'y'.repeat(4000),
        })
      )} });`
    );
    // No real network or retry sleeps: timers are immediate only in this isolated test process.
    writeFileSync(preload, '\nglobalThis.setTimeout = (fn) => { fn(); return 0; };\n', {
      flag: 'a',
    });
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
        'aarch64-apple-darwin',
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
    expect(result.target).toBe('aarch64-apple-darwin');
    expect(result.failed).toHaveLength(1);
    const failure = result.failed[0];
    expect(failure.check).toBe('tmt upgrade');
    expect(failure.reason.length).toBeLessThanOrEqual(500);
    expect(failure.detail.length).toBeLessThanOrEqual(6000);
    for (const text of [
      'failed after 3 attempts',
      'stdout: stdout-cause-',
      'stderr: stderr-cause-',
      '(4013 characters)',
    ]) {
      expect(failure.detail).toContain(text);
      expect(process.stderr).toContain(text);
    }
    expect(failure.detail).not.toContain('x'.repeat(2001));
    expect(failure.detail).not.toContain('y'.repeat(2001));
  });
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
