#!/usr/bin/env node
// Smoke test of a published alpha release through its public entry points, in an isolated home,
// state directory and prefix, so nothing touches a host installation. The pipeline's own checks
// read the release object and its attestation; this one installs from it, as a user does:
//   CLI        the public `releases/latest/download/install.sh` embeds the tag's version, installs
//              the tag, the installed `tmt` is the one PATH selects, the managed skills are the
//              tag's, and `tmt upgrade --channel alpha --json` reads the live metadata and says
//              the installation is current (or that a newer alpha has appeared since).
//   extension  the newest published CLI, installed the same way, runs `tmt extension install
//              <extension>` against the published release; `tmt extension list` reports the
//              tag's version and no CLI link appears.
// A failure is a result, never a rollback. Only data of the release is read from `--source`, a
// checkout of the tag; none of its code runs here.
//   node verify-public-install.mjs --product P --tag TAG --source DIR [--target T] [--result-file F]
import { spawnSync } from 'node:child_process';
import {
  appendFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { isReleased, parseComponentMap } from './ci-scope.mjs';
import { runPackedCommand } from './packed-command.mjs';
import { compareVersions, isAlphaVersion, versionOfTag } from './release-versions.mjs';

const ATTEMPTS = 2;
const MAX_WAIT_MS = 5 * 60_000;
const COMPONENTS = fileURLToPath(new URL('../../.github/components.json', import.meta.url));

export const installerUrl = (repository) =>
  `https://github.com/${repository}/releases/latest/download/install.sh`;

/** `text` as one bounded line, so a result is safe to put in a log, a summary or an issue. */
function oneLine(text, limit = 300) {
  const line = String(text)
    .replace(/\p{Cc}+/gu, ' ')
    .replace(/ {2,}/g, ' ')
    .trim();
  return line.length > limit ? `${line.slice(0, limit - 3)}...` : line;
}

async function fetchText(url) {
  const response = await fetch(url, { signal: AbortSignal.timeout(60_000), redirect: 'follow' });
  if (!response.ok) throw new Error(`HTTP ${response.status} for ${url}`);
  return response.text();
}

/** Only the native acquisition diagnostic is retryable; HTTP status alone is not evidence. */
function rateLimitDiagnostic(error) {
  const result = error.cause;
  if (!result || result.status !== 1 || result.stderr !== '') return null;
  try {
    const { error: failure } = JSON.parse(result.stdout);
    if (!['NATIVE_UPGRADE_FAILED', 'EXTENSION_INSTALL_FAILED'].includes(failure?.code)) return null;
    const cause = failure.cause;
    if (
      typeof cause !== 'string' ||
      !/^GitHub API rate limit: reset\/earliest retry time [^\n]+; the (?:required wait exceeds the remaining deadline|single retry was exhausted)\. Retry later or optionally set GITHUB_TOKEN\.$/.test(
        cause
      )
    )
      return null;
    return cause;
  } catch {
    return null;
  }
}

/** Retry only the failed acquisition, at most once and with at most five minutes of waiting. */
async function withAttempts(label, step, { wait = sleep, now = Date.now } = {}) {
  for (let attempt = 1; attempt <= ATTEMPTS; attempt += 1) {
    try {
      return await step();
    } catch (error) {
      const diagnostic = rateLimitDiagnostic(error);
      if (!diagnostic) throw error;
      const epoch = diagnostic.match(/\(UTC epoch (\d+)\)/)?.[1];
      const retryAtMs = epoch === undefined ? NaN : Number(epoch) * 1000;
      const waitMs = Math.max(1000, retryAtMs - now() + 1000);
      const reason =
        attempt === ATTEMPTS
          ? `attempt bound exceeded (${ATTEMPTS} attempts)`
          : !Number.isSafeInteger(retryAtMs)
            ? 'reset time unavailable'
            : waitMs > MAX_WAIT_MS
              ? `wait bound exceeded (${MAX_WAIT_MS / 1000} seconds)`
              : '';
      if (reason) {
        const failure = new Error(
          `Public install infrastructure: ${label}: ${reason}; ${diagnostic}`
        );
        failure.infrastructure = 'github-api-rate-limit';
        throw failure;
      }
      await wait(waitMs);
    }
  }
}

const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

/** `file` with its symlinks resolved, or as given when it does not exist. */
function resolved(file) {
  try {
    return realpathSync(file);
  } catch {
    return file;
  }
}

/** The first `name` executable on `searchPath`, which is what a shell would run. */
function findOnPath(name, searchPath) {
  for (const directory of searchPath.split(path.delimiter).filter(Boolean)) {
    const candidate = path.join(directory, name);
    if (existsSync(candidate)) return candidate;
  }
  return null;
}

/** Skill directories (those with a `SKILL.md`) of `directory`, mapped to the file's text. */
function skillsOf(directory) {
  if (!existsSync(directory)) return new Map();
  return new Map(
    readdirSync(directory)
      .filter((name) => existsSync(path.join(directory, name, 'SKILL.md')))
      .sort()
      .map((name) => [name, readFileSync(path.join(directory, name, 'SKILL.md'), 'utf8')])
  );
}

/**
 * Installs and checks `tag` of `product` from the public release, under `root`, and returns the
 * results, one per check, stopping at the first that fails (the later ones depend on it).
 * `source` is a checkout of the tag, `fetch` the one network read of this script, `wait` the pause
 * between attempts and `systemPath` the directories after the prefix on the isolated PATH.
 */
export async function smokeRelease({
  product,
  tag,
  source,
  repository,
  root,
  fetch: read = fetchText,
  wait = sleep,
  now = Date.now,
  systemPath = ['/usr/bin', '/bin', '/usr/sbin', '/sbin'],
}) {
  const version = versionOfTag(tag, product);
  const [home, state, tmp, prefix] = ['home', 'state', 'tmp', 'prefix'].map((name) => {
    const directory = path.join(root, name);
    mkdirSync(directory, { recursive: true });
    return directory;
  });
  const binary = path.join(prefix, 'bin', 'tmt');
  const env = {
    HOME: home,
    TMUX_TEAM_HOME: state,
    TMPDIR: tmp,
    PATH: [path.join(prefix, 'bin'), ...systemPath].join(path.delimiter),
    LANG: 'C',
    CI: 'true',
  };
  const tmt = (args, { timeoutMs = 120_000 } = {}) =>
    runPackedCommand(binary, args, { cwd: root, env, timeoutMs });
  const results = [];
  const check = async (name, step) => {
    try {
      results.push({ check: name, ok: true, reason: oneLine((await step()) ?? '') });
      return true;
    } catch (error) {
      const reason = oneLine(error.message, 500);
      // Keep the packed runner's bounded command/streams instead of losing them in the short reason.
      const detail = String(error.message)
        .replace(/[\p{Cc}\p{Zl}\p{Zp}]/gu, (character) => (character === '\n' ? '\n' : ' '))
        .slice(0, 6000);
      results.push({
        check: name,
        ok: false,
        reason,
        ...(error.infrastructure ? { infrastructure: error.infrastructure } : {}),
        ...(detail === reason ? {} : { detail }),
      });
      return false;
    }
  };
  const isCli = product === 'cli';

  let installer = '';
  const fetched = await check('public installer', async () => {
    for (let attempt = 1; attempt <= 3; attempt += 1) {
      installer = await read(installerUrl(repository));
      const embedded = /^\s*version='([^']+)'$/m.exec(installer)?.[1];
      if (!embedded) throw new Error('the installer names no version');
      if (!isCli || embedded === version) break;
      const mismatch = `the latest installer is for ${embedded}, not ${version}`;
      // The published latest entry can briefly lag. Only an older alpha is that condition;
      // download errors, malformed data and an unexpected newer version fail immediately.
      const lagging = isAlphaVersion(embedded) && compareVersions(embedded, version) < 0;
      if (!lagging || attempt === 3) throw new Error(mismatch);
      await wait(20_000);
    }
    return `embeds ${/^\s*version='([^']+)'$/m.exec(installer)[1]}`;
  });
  if (!fetched) return results;

  const installed = await check('install', async () => {
    const result = spawnSync('sh', ['-s', '--', '--prefix', prefix, '--no-setup'], {
      input: installer,
      env,
      cwd: root,
      encoding: 'utf8',
      timeout: 300_000,
    });
    if (result.error) throw result.error;
    if (result.status !== 0)
      throw new Error(`the installer exited ${result.status}: ${oneLine(result.stderr)}`);
  });
  if (!installed) return results;

  const cliVersion = /^\s*version='([^']+)'$/m.exec(installer)[1];
  const selected = await check('PATH selects the installed tmt', async () => {
    const found = findOnPath('tmt', env.PATH);
    if (!found || resolved(found) !== resolved(binary)) {
      throw new Error(`PATH selects ${found ?? 'no tmt'}, not ${binary}`);
    }
  });
  if (!selected) return results;
  if (
    !(await check('installed version', async () => {
      const actual = tmt(['--version']).trim();
      if (actual !== cliVersion) throw new Error(`tmt --version is ${actual}, not ${cliVersion}`);
      return actual;
    }))
  )
    return results;

  if (isCli) {
    if (
      !(await check('managed skills', async () => {
        const expected = skillsOf(path.join(source, 'skills'));
        const actual = skillsOf(path.join(home, '.agents', 'skills'));
        const names = (skills) => [...skills.keys()].join(', ') || 'none';
        if (names(expected) !== names(actual)) {
          throw new Error(
            `installed skills are ${names(actual)}, the release has ${names(expected)}`
          );
        }
        for (const [name, text] of expected) {
          if (actual.get(name) !== text) throw new Error(`${name}/SKILL.md differs from ${tag}`);
        }
        return names(actual);
      }))
    )
      return results;
    await check('tmt upgrade', async () => {
      const stdout = await withAttempts(
        'tmt upgrade',
        () => tmt(['upgrade', '--channel', 'alpha', '--json']),
        { wait, now }
      );
      const report = JSON.parse(stdout);
      if (resolved(report.executable) !== resolved(binary)) {
        throw new Error(`it upgraded ${report.executable}, not ${binary}`);
      }
      if (report.pathWarning) throw new Error(`PATH warning: ${report.pathWarning}`);
      if (report.skills?.conflicts?.length) throw new Error('skill conflicts were reported');
      if (report.version === version && report.changed === false) return `${version} is current`;
      if (compareVersions(report.version, version) > 0 && isAlphaVersion(report.version)) {
        return `note: a newer alpha, ${report.version}, was published meanwhile and is installed`;
      }
      throw new Error(`it reports ${report.version} (changed: ${report.changed}), not ${version}`);
    });
    return results;
  }

  const extensionPrefix = path.join(root, 'extension prefix');
  const extensionInstalled = await check(`${product} install`, async () => {
    const report = JSON.parse(
      await withAttempts(
        `${product} install`,
        () =>
          tmt(
            [
              'extension',
              'install',
              product,
              '--yes',
              '--json',
              '--channel',
              'alpha',
              '--prefix',
              extensionPrefix,
            ],
            { timeoutMs: 300_000 }
          ),
        { wait, now }
      )
    );
    if (report.version !== version)
      throw new Error(`it installed ${report.version}, not ${version}`);
    return report.version;
  });
  if (!extensionInstalled) return results;
  await check(`${product} list`, async () => {
    const { extensions } = JSON.parse(
      tmt(['extension', 'list', '--json', '--prefix', extensionPrefix])
    );
    const listed = extensions.find(({ name }) => name === product);
    if (listed?.version !== version)
      throw new Error(`the list reports ${listed?.version}, not ${version}`);
    if (existsSync(path.join(extensionPrefix, 'bin', 'tmt'))) {
      throw new Error('an extension install must not create the CLI link');
    }
    return listed.version;
  });
  return results;
}

/** Markdown for the run summary. */
export function renderSmokeSummary({ tag, target, results }) {
  const lines = [`### Public install of \`${tag}\` (${target})`, ''];
  for (const { check, ok, reason, detail } of results) {
    lines.push(`- ${ok ? 'passed' : 'FAILED'} ${check}${reason ? `: ${reason}` : ''}`);
    if (detail) lines.push('', ...detail.split('\n').map((line) => `    ${line}`), '');
  }
  return `${lines.join('\n')}\n`;
}

async function main(argv, environment) {
  const { values } = parseArgs({
    args: argv,
    options: {
      product: { type: 'string' },
      tag: { type: 'string' },
      source: { type: 'string' },
      target: { type: 'string', default: process.platform },
      'result-file': { type: 'string', default: '' },
    },
  });
  for (const name of ['product', 'tag', 'source']) {
    if (!values[name]) throw new Error(`--${name} is required.`);
  }
  const version = versionOfTag(values.tag, values.product);
  if (!isAlphaVersion(version)) throw new Error(`${values.tag} is not an alpha release.`);
  const map = parseComponentMap(readFileSync(COMPONENTS, 'utf8'));
  if (!isReleased(map, values.product)) {
    throw new Error(
      `${values.product} is not released (release: false), so there is nothing to install.`
    );
  }
  const repository = environment.GITHUB_REPOSITORY;
  if (!repository) throw new Error('GITHUB_REPOSITORY is not set.');
  const root = realpathSync(mkdtempSync(path.join(os.tmpdir(), 'tmt public install ')));
  let results;
  try {
    results = await smokeRelease({
      product: values.product,
      tag: values.tag,
      source: path.resolve(values.source),
      repository,
      root,
    });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
  const summary = renderSmokeSummary({ tag: values.tag, target: values.target, results });
  process.stderr.write(summary);
  if (environment.GITHUB_STEP_SUMMARY) appendFileSync(environment.GITHUB_STEP_SUMMARY, summary);
  if (values['result-file']) {
    writeFileSync(
      values['result-file'],
      JSON.stringify({
        target: values.target,
        failed: results.filter(({ ok }) => !ok).map(({ ok: _ok, ...failure }) => failure),
      })
    );
  }
  if (results.some(({ ok }) => !ok)) process.exitCode = 1;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2), process.env).catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}
