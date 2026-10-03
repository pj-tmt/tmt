#!/usr/bin/env node
// Re-prove only classified public-install failures, after the originating release run completed.
// Artifacts are bounded data. Only default-branch tooling runs in this workflow's writer job.
import { appendFileSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { ghPublishApi, reportFailure, reportSmokeRecovery } from './release-publish.mjs';
import { isAlphaVersion, versionOfTag } from './release-versions.mjs';
import { parseRateLimitDiagnostic } from './verify-public-install.mjs';

const MAX_WAIT_MS = 60 * 60_000;
const TARGETS = {
  'aarch64-apple-darwin': 'macos-15',
  'x86_64-apple-darwin': 'macos-15-intel',
  'aarch64-unknown-linux-musl': 'ubuntu-24.04-arm',
  'x86_64-unknown-linux-musl': 'ubuntu-24.04',
};

/** Admit the result format written by the smoke owner, never code or arbitrary paths. */
function admitHost(host) {
  if (
    !host ||
    !['cli', 'office', 'squad'].includes(host.product) ||
    !Object.hasOwn(TARGETS, host.target)
  ) {
    throw new Error('Invalid public-install product or target.');
  }
  if (!isAlphaVersion(versionOfTag(host.tag, host.product))) throw new Error('Not an alpha tag.');
  if (!Array.isArray(host.failed) || host.failed.length > 10)
    throw new Error('Invalid host failures.');
  for (const failure of host.failed) {
    if (
      !failure ||
      typeof failure.check !== 'string' ||
      typeof failure.reason !== 'string' ||
      failure.check.length > 300 ||
      failure.reason.length > 500 ||
      /[\p{Cc}\p{Zl}\p{Zp}]/u.test(failure.check + failure.reason)
    ) {
      throw new Error('Invalid public-install failure.');
    }
  }
  return host;
}

/** At most 128 product-qualified host artifacts, with a 64 KiB bound on each JSON file. */
export function readRetryHosts(directory, prefix = 'smoke-failures-', runAttempt = 1) {
  const names = readdirSync(directory)
    .filter((name) => name.startsWith(prefix))
    .sort();
  if (names.length > 128) throw new Error('Public-install artifact count exceeded 128.');
  return names.flatMap((name) => {
    const file = path.join(directory, name, 'smoke-result.json');
    if (statSync(file).size > 64 * 1024) throw new Error('Public-install result exceeds 64 KiB.');
    const host = admitHost(JSON.parse(readFileSync(file, 'utf8')));
    if (name !== `${prefix}${host.product}-${host.tag}-${host.target}`) {
      throw new Error('Public-install artifact identity mismatch.');
    }
    return host.runAttempt === runAttempt ? [host] : [];
  });
}

/** Select only targets whose single failed check carries the smoke owner's exact diagnostic. */
export function planSmokeRetry(hosts, { now = Date.now() } = {}) {
  const groups = new Map();
  for (const raw of hosts) {
    const host = admitHost(raw);
    const key = `${host.product}/${host.tag}`;
    if (!groups.has(key))
      groups.set(key, { product: host.product, tag: host.tag, hosts: [], targets: [] });
    const group = groups.get(key);
    if (group.hosts.some(({ target }) => target === host.target))
      throw new Error('Duplicate host result.');
    group.hosts.push(host);
  }
  const include = [];
  const skipped = [];
  let waitUntilMs = now;
  for (const group of groups.values()) {
    if (group.hosts.length !== 4)
      throw new Error(`Incomplete original smoke evidence for ${group.tag}.`);
    for (const host of group.hosts) {
      if (host.failed.length !== 1) continue;
      const failure = host.failed[0];
      const diagnostic = parseRateLimitDiagnostic(failure.rateLimit?.diagnostic);
      const expectedCheck = host.product === 'cli' ? 'tmt upgrade' : `${host.product} install`;
      if (
        failure.infrastructure !== 'github-api-rate-limit' ||
        failure.check !== expectedCheck ||
        !diagnostic
      )
        continue;
      const resetAtMs = diagnostic.resetAtMs;
      const reason =
        resetAtMs === null || resetAtMs !== failure.rateLimit.resetAtMs
          ? 'reset time unavailable or inconsistent'
          : resetAtMs + 1000 - now > MAX_WAIT_MS
            ? 'reset wait exceeds 60 minutes'
            : '';
      if (reason) {
        skipped.push({ product: host.product, tag: host.tag, target: host.target, reason });
        continue;
      }
      group.targets.push(host.target);
      include.push({
        product: host.product,
        tag: host.tag,
        target: host.target,
        runner: TARGETS[host.target],
      });
      waitUntilMs = Math.max(waitUntilMs, resetAtMs + 1000);
    }
  }
  return {
    groups: [...groups.values()].filter(({ targets }) => targets.length > 0),
    matrix: { include },
    waitUntilMs,
    skipped,
  };
}

/** One bounded timer on Linux, outside release-<product>; all selected resets have elapsed after it. */
export async function waitForSmokeReset(
  plan,
  { now = Date.now, wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms)) } = {}
) {
  const waitMs = Math.max(0, plan.waitUntilMs - now());
  if (!Number.isSafeInteger(waitMs) || waitMs > MAX_WAIT_MS)
    throw new Error('Reset wait exceeds 60 minutes.');
  if (waitMs) await wait(waitMs);
  if (now() < plan.waitUntilMs) throw new Error('Reported reset has not elapsed.');
}

/** Reconcile with the original four hosts: partial retries cannot close an infrastructure issue. */
export function reportSmokeRetry({ api, plan, retried, originalRunUrl, retryRunUrl }) {
  const expected = new Set(
    plan.matrix.include.map(({ product, tag, target }) => `${product}/${tag}/${target}`)
  );
  const replacements = new Map();
  for (const raw of retried) {
    const host = admitHost(raw);
    const key = `${host.product}/${host.tag}/${host.target}`;
    if (!expected.has(key) || replacements.has(key))
      throw new Error('Unexpected or duplicate retry result.');
    replacements.set(key, host);
  }
  const conclusions = [];
  for (const group of plan.groups) {
    const results = [];
    for (const original of group.hosts) {
      const selected = group.targets.includes(original.target);
      const replacement = replacements.get(`${group.product}/${group.tag}/${original.target}`);
      const host = selected ? replacement : original;
      if (!host) {
        results.push({
          check: `public install (${original.target})`,
          ok: false,
          reason: 'retry host evidence is missing; see both runs',
        });
        continue;
      }
      for (const { check, reason, infrastructure } of host.failed) {
        results.push({
          check: `${check} (${host.target})`,
          ok: false,
          reason,
          ...(infrastructure === 'github-api-rate-limit' ? { infrastructure } : {}),
        });
      }
    }
    if (results.length) {
      reportFailure({ api, tag: group.tag, results, runUrl: retryRunUrl, originalRunUrl });
      conclusions.push({ tag: group.tag, ok: false });
    } else {
      reportSmokeRecovery({ api, tag: group.tag, originalRunUrl, retryRunUrl });
      conclusions.push({ tag: group.tag, ok: true });
    }
  }
  return conclusions;
}

async function main(argv, env) {
  const [command, ...args] = argv;
  const { values } = parseArgs({
    args,
    options: {
      directory: { type: 'string' },
      plan: { type: 'string' },
      'original-run-url': { type: 'string' },
      'retry-run-url': { type: 'string' },
    },
  });
  if (!values.plan) throw new Error('--plan is required.');
  if (command === 'plan') {
    if (!values.directory) throw new Error('--directory is required.');
    const attempt = Number(env.SOURCE_RUN_ATTEMPT);
    if (!Number.isSafeInteger(attempt) || attempt < 1)
      throw new Error('SOURCE_RUN_ATTEMPT is required.');
    const plan = planSmokeRetry(readRetryHosts(values.directory, 'smoke-failures-', attempt));
    writeFileSync(values.plan, JSON.stringify(plan));
    appendFileSync(
      env.GITHUB_OUTPUT,
      `matrix=${JSON.stringify(plan.matrix)}\nretry=${plan.matrix.include.length > 0}\n`
    );
    process.stderr.write(
      `Selected ${plan.matrix.include.length} targets; skipped ${JSON.stringify(plan.skipped)}\n`
    );
  } else {
    // The plan is generated and kept by the trusted planner job, never by the installed product.
    const plan = JSON.parse(readFileSync(values.plan, 'utf8'));
    if (command === 'wait') await waitForSmokeReset(plan);
    else if (command === 'report') {
      for (const key of ['directory', 'original-run-url', 'retry-run-url'])
        if (!values[key]) throw new Error(`--${key} is required.`);
      const conclusions = reportSmokeRetry({
        api: ghPublishApi({ repository: env.GITHUB_REPOSITORY }),
        plan,
        retried: readRetryHosts(
          values.directory,
          'smoke-retry-',
          Number(env.GITHUB_RUN_ATTEMPT ?? '1')
        ),
        originalRunUrl: values['original-run-url'],
        retryRunUrl: values['retry-run-url'],
      });
      process.stderr.write(`${JSON.stringify(conclusions)}\n`);
      if (conclusions.some(({ ok }) => !ok)) process.exitCode = 1;
    } else throw new Error('Usage: public-install-retry.mjs plan|wait|report --plan FILE ...');
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2), process.env).catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}
