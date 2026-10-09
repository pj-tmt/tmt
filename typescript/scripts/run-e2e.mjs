#!/usr/bin/env node

import { spawn } from 'node:child_process';
import process from 'node:process';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';
import {
  dependencyCacheKey,
  inspectDependencyCache,
  renderDependencyDockerfile,
} from './e2e-dependency-cache.mjs';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const image = `tmux-team-e2e:${process.pid}-${Date.now().toString(36)}`;
const dockerfile = path.join(repoRoot, 'typescript', 'test', 'e2e', 'Dockerfile');
let activeChild;
let interrupted = false;

function run(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd: repoRoot, stdio: 'inherit', ...options });
    activeChild = child;
    let settled = false;
    child.once('error', (error) => {
      if (settled) return;
      settled = true;
      if (activeChild === child) activeChild = undefined;
      reject(error);
    });
    child.once('close', (code) => {
      if (settled) return;
      settled = true;
      if (activeChild === child) activeChild = undefined;
      resolve(code ?? 1);
    });
  });
}

function forwardSignal(signal) {
  interrupted = true;
  activeChild?.kill(signal);
}

process.once('SIGINT', () => forwardSignal('SIGINT'));
process.once('SIGTERM', () => forwardSignal('SIGTERM'));

async function removeImage() {
  try {
    await run('docker', ['image', 'rm', '--force', image], { stdio: 'ignore' });
  } catch {
    // Preserve the primary build/test failure when cleanup cannot run.
  }
}

/**
 * Optional scoping of the suite to named files. The value becomes a shell word list
 * inside the image, so only plain file names are accepted.
 */
const FILE_LIST = /^[A-Za-z0-9._-]+( [A-Za-z0-9._-]+)*$/;
const ADAPTER_FLAG = /^[01]$/;
/** Cargo's parallel job limit for the image's native builds: a positive count or `default`. */
const CARGO_JOBS = /^(?:[1-9][0-9]*|default)$/;

async function main() {
  const files = process.env.TMT_E2E_FILES ?? '';
  if (files !== '' && !FILE_LIST.test(files)) {
    console.error('TMT_E2E_FILES must be a space-separated list of plain file names.');
    return 2;
  }
  const adapterTests = process.env.TMT_E2E_ADAPTER_TESTS ?? '';
  if (adapterTests !== '' && !ADAPTER_FLAG.test(adapterTests)) {
    console.error('TMT_E2E_ADAPTER_TESTS must be 0 or 1.');
    return 2;
  }
  const cargoJobs = process.env.CARGO_BUILD_JOBS ?? '';
  if (cargoJobs !== '' && !CARGO_JOBS.test(cargoJobs)) {
    console.error('CARGO_BUILD_JOBS must be a positive integer or default.');
    return 2;
  }
  let temporary;
  try {
    let buildFile = dockerfile;
    let dependencyArgs = [];
    const bundle = process.env.TMT_E2E_DEPENDENCY_CACHE;
    if (process.env.GITHUB_ACTIONS === 'true' && process.env.CI === 'true' && bundle) {
      const admitted = inspectDependencyCache(bundle, dependencyCacheKey(repoRoot));
      console.error(`E2E dependency cache: ${admitted.reason}`);
      if (admitted.usable) {
        const rendered = renderDependencyDockerfile(fs.readFileSync(dockerfile, 'utf8'));
        temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-e2e-dependencies-'));
        buildFile = path.join(temporary, 'Dockerfile');
        fs.writeFileSync(buildFile, rendered);
        dependencyArgs = ['--load', '--build-context', `tmtdeps=${path.resolve(bundle)}`];
      }
    }
    const buildStatus = await run('docker', [
      ...(temporary ? ['buildx', 'build'] : ['build']),
      ...dependencyArgs,
      '--tag',
      image,
      ...(cargoJobs === '' ? [] : ['--build-arg', `CARGO_BUILD_JOBS=${cargoJobs}`]),
      '--file',
      buildFile,
      repoRoot,
    ]);
    if (buildStatus !== 0) return interrupted ? 130 : buildStatus;
    if (interrupted) return 130;
    // Selectors name container-visible executables. Forward values as single
    // argv entries; never translate host paths or evaluate shell fragments.
    const selection = ['TMT_TEST_CLI', 'TMT_TEST_PEER_CLI'].flatMap((key) =>
      process.env[key] === undefined ? [] : ['--env', `${key}=${process.env[key]}`]
    );
    const scope = [
      ...(files === '' ? [] : ['--env', `TMT_E2E_FILES=${files}`]),
      ...(adapterTests === '' ? [] : ['--env', `TMT_E2E_ADAPTER_TESTS=${adapterTests}`]),
    ];
    const testStatus = await run('docker', [
      'run',
      '--rm',
      '--init',
      '--network',
      'none',
      ...selection,
      ...scope,
      image,
    ]);
    return interrupted ? 130 : testStatus;
  } catch (error) {
    if (error?.code === 'ENOENT') {
      console.error('E2E setup failed: Docker is required but was not found on PATH.');
    } else {
      console.error(`E2E setup failed: ${error instanceof Error ? error.message : String(error)}`);
    }
    return 1;
  } finally {
    await removeImage();
    if (temporary) {
      try {
        fs.rmSync(temporary, { recursive: true, force: true });
      } catch {
        /* Preserve the primary build/test failure when cleanup cannot run. */
      }
    }
  }
}

main().then((code) => {
  process.exitCode = code;
});
