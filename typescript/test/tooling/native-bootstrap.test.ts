import { createHash } from 'node:crypto';
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vitest';
import * as tar from 'tar';
import { runCli, withSandbox, type Sandbox } from '../support/cli-process.js';

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const { generateNativeBootstrap } = (await import(
  pathToFileURL(path.join(repositoryRoot, 'scripts', 'native-bootstrap.mjs')).href
)) as unknown as {
  generateNativeBootstrap: (
    manifestFile: string,
    archiveDirectory: string,
    planFile?: string
  ) => Promise<string>;
};

// The CLI archive's files, from the artifact policy that owns the list.
const { runtimeFiles, companionFiles } = (await import(
  pathToFileURL(path.join(repositoryRoot, 'scripts', 'native-artifact-policy.mjs')).href
)) as unknown as {
  runtimeFiles: () => string[];
  companionFiles: () => string[];
};
const REQUIRED_FILES = runtimeFiles();
const COMPANIONS = companionFiles();

function nativeTarget(): string {
  const architecture = process.arch === 'arm64' ? 'aarch64' : 'x86_64';
  const platform = process.platform === 'darwin' ? 'apple-darwin' : 'unknown-linux-musl';
  return `${architecture}-${platform}`;
}

type Fixture = {
  readonly archive: string;
  readonly manifest: string;
  readonly version: string;
  readonly target: string;
};

async function createFixture(
  sandbox: Sandbox,
  options: { readonly version?: string; readonly target?: string } = {}
): Promise<Fixture> {
  const version = options.version ?? '5.0.0-alpha.1';
  const target = options.target ?? nativeTarget();
  const name = `tmux-team-${version}-${target}.tar.gz`;
  const rootName = name.slice(0, -'.tar.gz'.length);
  const fixtureRoot = mkdtempSync(path.join(sandbox.root, 'bootstrap-fixture-'));
  const tree = path.join(fixtureRoot, 'tree');
  const root = path.join(tree, rootName);
  const archive = path.join(fixtureRoot, name);
  const manifest = path.join(fixtureRoot, 'dist-manifest.json');
  mkdirSync(root, { recursive: true });
  writeFileSync(path.join(root, 'tmt'), stubExecutable(), { mode: 0o755 });
  chmodSync(path.join(root, 'tmt'), 0o755);
  for (const file of REQUIRED_FILES.slice(1)) {
    // A companion is never run here; it only has to be an executable file.
    const companion = COMPANIONS.includes(file);
    writeFileSync(path.join(root, file), companion ? '#!/bin/sh\nexit 0\n' : `${file}\n`, {
      mode: companion ? 0o755 : 0o644,
    });
  }
  await tar.c({ cwd: tree, file: archive, gzip: true }, [rootName]);
  const checksum = createHash('sha256').update(readFileSync(archive)).digest('hex');
  writeFileSync(
    manifest,
    `${JSON.stringify({
      artifacts: {
        [name]: {
          kind: 'executable-zip',
          name,
          target_triples: [target],
          checksums: { sha256: checksum },
          assets: REQUIRED_FILES.map((file) => ({ path: file })),
        },
      },
      releases: [{ app_name: 'tmt-cli', app_version: version, artifacts: [name] }],
    })}\n`
  );
  return { archive, manifest, version, target };
}

function stubExecutable(): string {
  return `#!/bin/sh
set -eu
log="\${TMT_BOOTSTRAP_STUB_LOG:?}"
command=\${1-}
printf 'command=%s\\n' "$command" >> "$log"
shift || true
prefix=
for argument do
  printf 'arg=%s\\n' "$argument" >> "$log"
  if [ "$argument" = --prefix ]; then prefix=\${2:?}; fi
  shift
done
case "$command" in
  __native-install)
    mkdir -p "$prefix/bin"
    cp "$0" "$prefix/bin/tmt"
    chmod 755 "$prefix/bin/tmt"
    printf '%s\\n' '{"installed":true}'
    ;;
  install)
    if [ "\${TMT_BOOTSTRAP_SKILL_FAILURE-}" = 1 ]; then exit 7; fi
    printf '%s\\n' '{"installed":true}'
    ;;
  setup)
    if [ -t 0 ]; then printf 'stdin=terminal\\n' >> "$log"; fi
    printf 'path=%s\\n' "$(command -v tmt)" >> "$log"
    ;;
  *) exit 64 ;;
esac
`;
}

function writeCurlFixture(directory: string): void {
  const script = `#!/bin/sh
set -eu
: "\${TMT_BOOTSTRAP_CURL_LOG:?}"
printf '%s\\n' "$@" >> "$TMT_BOOTSTRAP_CURL_LOG.args"
output=
url=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output|-o) output=$2; shift 2 ;;
    --proto|--proto-redir|--max-redirs|--connect-timeout|--max-time|--max-filesize) shift 2 ;;
    --disable|--fail|--silent|--show-error|--location|--tlsv1.2|-f|-s|-S|-L|-fsSL|-sSfL) shift ;;
    *) url=$1; shift ;;
  esac
done
printf '%s\\n' "$url" >> "$TMT_BOOTSTRAP_CURL_LOG"
case "$url" in
  *dist-manifest.json) source=\${TMT_BOOTSTRAP_MANIFEST:?} ;;
  *.tar.gz) source=\${TMT_BOOTSTRAP_ARCHIVE:?} ;;
  *) exit 22 ;;
esac
if [ "\${TMT_BOOTSTRAP_CURL_FAILURE-}" = 1 ]; then
  printf 'partial download' > "$output"
  exit 56
fi
if [ "\${TMT_BOOTSTRAP_INTERRUPT-}" = 1 ]; then
  printf 'partial download' > "$output"
  kill -TERM "$PPID"
  exit 143
fi
cp "$source" "$output"
if [ "\${TMT_BOOTSTRAP_SHORT-}" = 1 ]; then : > "$output"; fi
if [ "\${TMT_BOOTSTRAP_CORRUPT-}" = manifest ] && printf '%s' "$url" | grep -q dist-manifest; then
  printf 'X' | dd of="$output" bs=1 count=1 conv=notrunc 2>/dev/null
elif [ "\${TMT_BOOTSTRAP_CORRUPT-}" = archive ] && printf '%s' "$url" | grep -q tar.gz; then
  printf 'X' | dd of="$output" bs=1 count=1 conv=notrunc 2>/dev/null
fi
`;
  const target = path.join(directory, 'curl');
  writeFileSync(target, script, { mode: 0o755 });
  chmodSync(target, 0o755);
}

function writeUnameFixture(directory: string): void {
  const target = path.join(directory, 'uname');
  writeFileSync(target, '#!/bin/sh\nprintf "%s\\n" "Plan9"\n', { mode: 0o755 });
  chmodSync(target, 0o755);
}

async function runBootstrap(
  sandbox: Sandbox,
  script: string,
  fixture: Fixture,
  args: readonly string[] = [],
  options: {
    readonly prefix?: string;
    readonly corrupt?: 'manifest' | 'archive';
    readonly curlFailure?: boolean;
    readonly interrupted?: boolean;
    readonly shortDownload?: boolean;
    readonly skillFailure?: boolean;
    readonly unsupportedPlatform?: boolean;
    readonly endpointOverride?: string;
    /** Run under a pseudo-terminal, as a person at a shell would. */
    readonly terminal?: boolean;
    /** System directories on PATH after the fake tools; the host's by default. */
    readonly systemPath?: string;
  } = {}
) {
  const stage = mkdtempSync(path.join(sandbox.root, 'bootstrap-tmp-'));
  const fakeBin = mkdtempSync(path.join(sandbox.root, 'bootstrap-tools-'));
  const installer = path.join(sandbox.root, 'native-bootstrap.sh');
  writeFileSync(installer, script, { mode: 0o700 });
  chmodSync(installer, 0o700);
  writeCurlFixture(fakeBin);
  if (options.unsupportedPlatform) writeUnameFixture(fakeBin);
  const prefix = options.prefix ?? path.join(sandbox.home, '.local');
  const log = path.join(sandbox.root, 'stub.log');
  const curlLog = path.join(sandbox.root, 'curl.log');
  const systemPath = options.systemPath ?? process.env.PATH ?? '/usr/bin:/bin';
  const installerArgs = ['--prefix', prefix, ...args];
  // `script` gives the installer a pseudo-terminal: BSD and util-linux differ.
  const quoted = ['/bin/sh', installer, ...installerArgs]
    .map((word) => `'${word.replaceAll("'", "'\\''")}'`)
    .join(' ');
  const cli = !options.terminal
    ? { executable: '/bin/sh', args: [installer] }
    : process.platform === 'darwin'
      ? { executable: '/usr/bin/script', args: ['-q', '/dev/null', '/bin/sh', installer] }
      : { executable: 'script', args: ['-qec', quoted, '/dev/null'] };
  const result = await runCli(
    {
      ...sandbox,
      cli,
      env: {
        ...sandbox.env,
        CI: '',
        PATH: `${fakeBin}:${systemPath}`,
        TMPDIR: stage,
        TMT_BOOTSTRAP_STUB_LOG: log,
        TMT_BOOTSTRAP_CURL_LOG: curlLog,
        TMT_BOOTSTRAP_MANIFEST: fixture.manifest,
        TMT_BOOTSTRAP_ARCHIVE: fixture.archive,
        ...(options.corrupt === undefined ? {} : { TMT_BOOTSTRAP_CORRUPT: options.corrupt }),
        ...(options.curlFailure ? { TMT_BOOTSTRAP_CURL_FAILURE: '1' } : {}),
        ...(options.interrupted ? { TMT_BOOTSTRAP_INTERRUPT: '1' } : {}),
        ...(options.shortDownload ? { TMT_BOOTSTRAP_SHORT: '1' } : {}),
        ...(options.skillFailure ? { TMT_BOOTSTRAP_SKILL_FAILURE: '1' } : {}),
        ...(options.endpointOverride
          ? { TMT_NATIVE_BOOTSTRAP_ENDPOINT: options.endpointOverride }
          : {}),
      },
    },
    options.terminal && process.platform !== 'darwin' ? [] : installerArgs,
    { deadlineMs: 15_000 }
  );
  return { result, prefix, log: existsSync(log) ? readFileSync(log, 'utf8') : '', stage, curlLog };
}

function expectCleanStage(stage: string): void {
  expect(readdirSync(stage)).toEqual([]);
}

describe('native curl bootstrap', () => {
  it('checks every archive in a multi-platform release before host selection', async () => {
    await withSandbox(async (sandbox) => {
      const targets = [
        'aarch64-apple-darwin',
        'x86_64-apple-darwin',
        'aarch64-unknown-linux-musl',
        'x86_64-unknown-linux-musl',
      ];
      const directory = mkdtempSync(path.join(sandbox.root, 'release-bundle-'));
      const combined = {
        artifacts: {} as Record<string, unknown>,
        releases: [
          { app_name: 'tmt-cli', app_version: '5.0.0-alpha.1', artifacts: [] as string[] },
        ],
      };
      let host: Fixture | undefined;
      let foreignArchive = '';
      for (const target of targets) {
        const fixture = await createFixture(sandbox, { target });
        const metadata = JSON.parse(readFileSync(fixture.manifest, 'utf8')) as typeof combined;
        Object.assign(combined.artifacts, metadata.artifacts);
        combined.releases[0].artifacts.push(path.basename(fixture.archive));
        const archive = path.join(directory, path.basename(fixture.archive));
        copyFileSync(fixture.archive, archive);
        if (target === nativeTarget()) host = { ...fixture, archive };
        else foreignArchive = archive;
      }
      const manifest = path.join(directory, 'dist-manifest.json');
      writeFileSync(manifest, JSON.stringify(combined));
      const plan = path.join(directory, 'plan.json');
      writeFileSync(plan, JSON.stringify(combined));
      const script = await generateNativeBootstrap(manifest, directory, plan);
      expect(host).toBeDefined();
      const run = await runBootstrap(sandbox, script, { ...host!, manifest }, ['--no-skill']);
      expect(run.result.status).toBe(0);
      expect(readFileSync(run.curlLog, 'utf8')).toContain(path.basename(host!.archive));
      expectCleanStage(run.stage);

      const incomplete = structuredClone(combined);
      delete incomplete.artifacts[path.basename(foreignArchive)];
      writeFileSync(manifest, JSON.stringify(incomplete));
      await expect(generateNativeBootstrap(manifest, directory, plan)).rejects.toThrow(
        'Release must contain every planned native artifact'
      );
      writeFileSync(manifest, JSON.stringify(combined));

      // Even an archive for a different CPU/OS is part of the release trust input.
      const corrupt = readFileSync(foreignArchive);
      corrupt[0] ^= 1;
      writeFileSync(foreignArchive, corrupt);
      await expect(generateNativeBootstrap(manifest, directory)).rejects.toThrow(
        'Native archive checksum mismatch'
      );
    });
  });

  it('generates a release-specific script from verified cargo-dist artifacts', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = await createFixture(sandbox);
      const script = await generateNativeBootstrap(fixture.manifest, path.dirname(fixture.archive));
      expect(script).toContain(fixture.version);
      expect(script).toContain(fixture.target);
      expect(script).toContain(path.basename(fixture.archive));
      expect(script).toContain(
        createHash('sha256').update(readFileSync(fixture.archive)).digest('hex')
      );
      expect(script).toContain('https://github.com/pj-tmt/tmt/releases/download/');
      expect(script).not.toContain('endpointOverride');
    });
  });

  it(
    'publishes through __native-install, handles spaces and pinning, and skips skills on request',
    { timeout: 20_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const fixture = await createFixture(sandbox);
        const script = await generateNativeBootstrap(
          fixture.manifest,
          path.dirname(fixture.archive)
        );
        const prefix = path.join(sandbox.root, 'native prefix with spaces');
        mkdirSync(path.join(prefix, 'bin'), { recursive: true });
        writeFileSync(path.join(prefix, 'bin', 'npm-owned'), 'preserve me\n');
        // Only system directories: no other tmt, as on a fresh machine.
        const run = await runBootstrap(sandbox, script, fixture, ['--pin', '--no-skill'], {
          prefix,
          systemPath: '/usr/bin:/bin',
        });

        expect(run.result.status).toBe(0);
        expect(run.result.stderr).toBe('');
        expect(run.result.stdout).toContain(`Native tmt is installed at ${prefix}/bin/tmt.`);
        expect(run.result.stdout).toContain(`${prefix}/bin is not in PATH yet.`);
        expect(run.result.stdout).not.toContain('another installation');
        expect(run.log).toContain('command=__native-install');
        expect(run.log).toContain(`arg=${prefix}`);
        expect(run.log).toContain('arg=--pin\n');
        expect(run.log).toContain('arg=--channel\narg=alpha\n');
        expect(run.log).not.toContain('command=install');
        const curlArgs = readFileSync(`${run.curlLog}.args`, 'utf8');
        expect(curlArgs).toMatch(/^--disable\n/);
        expect(curlArgs).toContain('--proto\n=https\n--proto-redir\n=https\n');
        expect(curlArgs).toContain('--max-redirs\n3\n');
        expect(curlArgs).toContain('--max-time\n60\n');
        expect(curlArgs).not.toContain('--insecure');
        expect(readFileSync(path.join(prefix, 'bin', 'npm-owned'), 'utf8')).toBe('preserve me\n');
        expect(existsSync(path.join(prefix, 'bin', 'tmt'))).toBe(true);
        expectCleanStage(run.stage);
      });
    }
  );

  it(
    'runs skills through the absolute published command and preserves partial success',
    { timeout: 20_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const fixture = await createFixture(sandbox);
        const script = await generateNativeBootstrap(
          fixture.manifest,
          path.dirname(fixture.archive)
        );
        const run = await runBootstrap(sandbox, script, fixture, [], { skillFailure: true });

        expect(run.result.status).toBe(1);
        expect(run.result.stderr).toContain('native binary is installed, but skill setup failed');
        expect(run.log).toContain('command=__native-install');
        expect(run.log).toContain('command=install');
        expect(existsSync(path.join(run.prefix, 'bin', 'tmt'))).toBe(true);
        expect(existsSync(sandbox.database)).toBe(false);
        expectCleanStage(run.stage);
      });
    }
  );

  it.each(['manifest', 'archive'] as const)(
    'does not execute an unverified %s',
    async (corrupt) => {
      await withSandbox(async (sandbox) => {
        const fixture = await createFixture(sandbox);
        const script = await generateNativeBootstrap(
          fixture.manifest,
          path.dirname(fixture.archive)
        );
        const run = await runBootstrap(sandbox, script, fixture, [], {
          corrupt,
          endpointOverride: 'https://evil.example.invalid/replacement',
        });

        expect(run.result.status).toBe(1);
        expect(run.result.stderr).toContain('Downloaded asset SHA-256 mismatch.');
        expect(run.log).toBe('');
        expect(existsSync(path.join(run.prefix, 'bin', 'tmt'))).toBe(false);
        const urls = readFileSync(run.curlLog, 'utf8').trim().split('\n');
        const base = `https://github.com/pj-tmt/tmt/releases/download/v${fixture.version}`;
        expect(urls).toEqual([
          `${base}/dist-manifest.json`,
          ...(corrupt === 'archive' ? [`${base}/${path.basename(fixture.archive)}`] : []),
        ]);
        expectCleanStage(run.stage);
      });
    }
  );

  it(
    'continues into guided setup only on a terminal, otherwise installs skills',
    { timeout: 30_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const fixture = await createFixture(sandbox);
        const script = await generateNativeBootstrap(
          fixture.manifest,
          path.dirname(fixture.archive)
        );
        const piped = await runBootstrap(sandbox, script, fixture);
        expect(piped.result.status, piped.result.stderr).toBe(0);
        expect(piped.log).toContain('command=install');
        expect(piped.log).not.toContain('command=setup');
        expect(piped.result.stdout).toContain('Run tmt setup to connect your agents.');

        const optedOut = await runBootstrap(sandbox, script, fixture, ['--no-setup'], {
          terminal: true,
        });
        expect(optedOut.result.status, optedOut.result.stdout).toBe(0);
        expect(optedOut.log).not.toContain('command=setup');

        const guided = await runBootstrap(sandbox, script, fixture, [], { terminal: true });
        expect(guided.result.status, guided.result.stdout).toBe(0);
        const setupLog = guided.log.slice(guided.log.lastIndexOf('command=setup'));
        expect(setupLog).toContain('command=setup');
        // The approval is read from the terminal, and the new tmt is on PATH.
        expect(setupLog).toContain('stdin=terminal');
        expect(setupLog).toContain(`path=${path.join(guided.prefix, 'bin', 'tmt')}`);
        // The log accumulates across runs: guided setup added no skill install.
        const installs = (log: string) => log.split('command=install').length - 1;
        expect(installs(guided.log)).toBe(installs(optedOut.log));
      });
    }
  );

  it('propagates a curl failure without invoking the downloaded executable', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = await createFixture(sandbox);
      const script = await generateNativeBootstrap(fixture.manifest, path.dirname(fixture.archive));
      const run = await runBootstrap(sandbox, script, fixture, [], { curlFailure: true });

      expect(run.result.status).toBe(56);
      expect(run.log).toBe('');
      expect(existsSync(path.join(run.prefix, 'bin', 'tmt'))).toBe(false);
      expect(readFileSync(run.curlLog, 'utf8').trim()).toBe(
        `https://github.com/pj-tmt/tmt/releases/download/v${fixture.version}/dist-manifest.json`
      );
      expectCleanStage(run.stage);
    });
  });

  it('rejects a short body and cleans a terminated partial download before publication', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = await createFixture(sandbox);
      const script = await generateNativeBootstrap(fixture.manifest, path.dirname(fixture.archive));
      const short = await runBootstrap(sandbox, script, fixture, [], { shortDownload: true });
      expect(short.result.status).toBe(1);
      expect(short.result.stderr).toContain('Downloaded asset size mismatch.');
      expect(short.log).toBe('');
      expectCleanStage(short.stage);
      const interrupted = await runBootstrap(sandbox, script, fixture, [], { interrupted: true });
      expect(interrupted.result.status).toBe(143);
      expect(interrupted.log).toBe('');
      expect(existsSync(path.join(interrupted.prefix, 'bin/tmt'))).toBe(false);
      expectCleanStage(interrupted.stage);
    });
  });

  it('rejects invalid options and unsupported hosts before downloading', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = await createFixture(sandbox);
      const script = await generateNativeBootstrap(fixture.manifest, path.dirname(fixture.archive));
      const invalid = await runBootstrap(sandbox, script, fixture, ['--unexpected']);
      expect(invalid.result.status).toBe(1);
      expect(invalid.result.stderr).toContain('Unknown installer option');
      expect(invalid.log).toBe('');
      expect(existsSync(invalid.curlLog)).toBe(false);

      const unsupported = await runBootstrap(sandbox, script, fixture, [], {
        unsupportedPlatform: true,
      });
      expect(unsupported.result.status).toBe(1);
      expect(unsupported.result.stderr).toContain('no verified artifact for Plan9:Plan9');
      expect(unsupported.log).toBe('');
      expect(existsSync(unsupported.curlLog)).toBe(false);
    });
  });

  it('does not install when the piped script is truncated at its final invocation', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = await createFixture(sandbox);
      const script = await generateNativeBootstrap(fixture.manifest, path.dirname(fixture.archive));
      const invocation = script.lastIndexOf('main "$@"');
      expect(invocation).toBeGreaterThan(0);
      const run = await runBootstrap(sandbox, script.slice(0, invocation + 4), fixture);
      expect(run.result.status).toBe(2);
      expect(run.result.stderr).toMatch(/syntax error|Syntax error/);
      expect(run.log).toBe('');
      expect(existsSync(run.curlLog)).toBe(false);
      expect(existsSync(run.prefix)).toBe(false);
    });
  });

  it('rejects duplicate targets and unsafe versions during generation', async () => {
    await withSandbox(async (sandbox) => {
      const first = await createFixture(sandbox);
      const second = await createFixture(sandbox, { version: '5.0.0-alpha.2' });
      const duplicate = JSON.parse(readFileSync(first.manifest, 'utf8')) as {
        artifacts: Record<string, unknown>;
        releases: Array<Record<string, unknown>>;
      };
      const secondManifest = JSON.parse(readFileSync(second.manifest, 'utf8')) as typeof duplicate;
      Object.assign(duplicate.artifacts, secondManifest.artifacts);
      duplicate.releases.push(...secondManifest.releases);
      writeFileSync(first.manifest, JSON.stringify(duplicate));
      await expect(
        generateNativeBootstrap(first.manifest, path.dirname(first.archive))
      ).rejects.toThrow('Duplicate bootstrap target');

      const unsafe = JSON.parse(readFileSync(first.manifest, 'utf8')) as typeof duplicate;
      const name = path.basename(first.archive);
      unsafe.artifacts = { [name]: unsafe.artifacts[name] };
      unsafe.releases = [
        { app_name: 'tmt-cli', app_version: '5.0.0;echo-pwned', artifacts: [name] },
      ];
      writeFileSync(first.manifest, JSON.stringify(unsafe));
      await expect(
        generateNativeBootstrap(first.manifest, path.dirname(first.archive))
      ).rejects.toThrow('Unsafe or unsupported bootstrap version');
    });
  });
});
