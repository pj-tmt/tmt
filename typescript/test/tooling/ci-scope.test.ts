import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  symlinkSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { listE2eFiles } from '../../scripts/e2e-shards.mjs';
import { readCargoWorkspace } from '../../scripts/cargo-workspace.mjs';
import {
  RUST_WORKERS,
  MAIN_FULL_JOBS,
  admitMainFull,
  mainFullGatePasses,
  notifyMainFullFailure,
  ciGatePasses,
  e2eGatePasses,
  explainCiSelection,
  globToRegExp,
  isReleased,
  nativeGatePasses,
  ownerOf,
  parseComponentMap,
  releasedComponentsForPath,
  releasedComponentNamesOfPath,
  readChangedCiAreas,
  renderSelectionEvidence,
  runCiScope,
  rustGatePasses,
  scopedChecks,
  selectCiAreas,
  selectNativeScope,
  selectColabApp,
  selectRemoteFirestore,
  selectNativeSyncEmulator,
  selectHostedReaderEmulator,
  selectColabFirestore,
  selectColabHarness,
  selectColabChrome,
  selectColabHosted,
  selectNativeNotices,
} from '../../scripts/ci-scope.mjs';

const { runPackedCommand } = await import(
  new URL('../../scripts/packed-command.mjs', import.meta.url).href
);
const { isComponentRetired } = (await import(
  new URL('../../scripts/native-release-policy.mjs', import.meta.url).href
)) as { isComponentRetired: (name: string) => boolean };

it('keeps the adapter environment runner on full native verification', () => {
  expect(selectNativeScope(['typescript/scripts/e2e-adapter-tests.mjs'])).toBe('full');
});

describe('complete trusted-main verification', () => {
  const context = {
    repository: 'pj-tmt/tmt',
    eventName: 'push',
    ref: 'refs/heads/main',
    sha: 'a'.repeat(40),
    runId: '42',
    runAttempt: '1',
  };
  const capture = () => {
    let text = '';
    return { write: (value: string) => (text += value), text: () => text };
  };
  const completeNeeds = () => {
    const stdout = capture();
    runCiScope(['main-full'], { cwd: '/unused', stdout, stderr: capture() });
    const needs: Record<string, { result: string; outputs?: Record<string, string> }> =
      Object.fromEntries(MAIN_FULL_JOBS.map((job) => [job, { result: 'success' }]));
    needs.changes.outputs = {
      ...Object.fromEntries(
        stdout
          .text()
          .trim()
          .split('\n')
          .map((line) => line.split('='))
      ),
      verify: 'true',
      macos: 'true',
      main_full_admitted: 'true',
    };
    return needs;
  };
  const job = (id: number, conclusion = 'success') => ({ id, name: `worker ${id}`, conclusion });

  it('selects both Rules suites and native sync, unlike weekly cache seeding', () => {
    let output = '';
    runCiScope(['main-full'], {
      cwd: '/no-diff-required',
      stdout: { write: (text: string) => (output += text) },
      stderr: { write: () => {} },
    });
    const selected = Object.fromEntries(
      output
        .trim()
        .split('\n')
        .map((line) => line.split('='))
    );
    expect(selected).toMatchObject({
      native: 'true',
      native_scope: 'full',
      native_notices: 'true',
      remote_firestore: 'true',
      colab_firestore: 'true',
      native_sync_emulator: 'true',
    });
  });

  it('admits current main once and skips a superseded event without retargeting it', () => {
    const reads: string[] = [];
    expect(
      admitMainFull(context, () => {
        reads.push('current');
        return context.sha;
      })
    ).toBe(true);
    expect(
      admitMainFull(context, () => {
        reads.push('newer');
        return 'b'.repeat(40);
      })
    ).toBe(false);
    expect(reads).toEqual(['current', 'newer']);
    const failure = new Error('API unavailable');
    expect(() =>
      admitMainFull(context, () => {
        throw failure;
      })
    ).toThrow(failure);
    expect(() => admitMainFull(context, () => null)).toThrow('no exact commit SHA');
  });

  it.each([
    { eventName: 'pull_request' },
    { eventName: 'merge_group' },
    { eventName: 'workflow_dispatch' },
    { eventName: 'schedule' },
    { ref: 'refs/heads/feature' },
    { repository: 'someone/tmt' },
    { sha: 'main' },
  ])('rejects non-authoritative context before any API access: %j', (changed) => {
    let reads = 0;
    const bad = { ...context, ...changed };
    expect(() =>
      admitMainFull(bad, () => {
        reads += 1;
        return context.sha;
      })
    ).toThrow('trusted main push');
    expect(mainFullGatePasses(bad, completeNeeds())).toBe(false);
    expect(() =>
      notifyMainFullFailure(bad, () => {
        reads += 1;
        return null;
      })
    ).toThrow('trusted main push');
    expect(reads).toBe(0);
  });

  it('requires all full flags, complete disjoint shards and the exact worker inventory', () => {
    expect(mainFullGatePasses(context, completeNeeds())).toBe(true);
    for (const key of [
      'verify',
      'macos',
      'main_full_admitted',
      'native',
      'native_notices',
      'remote_firestore',
      'colab_firestore',
      'colab_chrome',
      'native_sync_emulator',
    ]) {
      const needs = completeNeeds();
      needs.changes.outputs![key] = 'false';
      expect(mainFullGatePasses(context, needs), key).toBe(false);
    }
    for (const key of ['native_scope', 'e2e_shard_1', 'e2e_shard_2']) {
      const needs = completeNeeds();
      needs.changes.outputs![key] = '';
      expect(mainFullGatePasses(context, needs), key).toBe(false);
    }
    const extra = completeNeeds();
    extra.unclassified = { result: 'success' };
    expect(mainFullGatePasses(context, extra)).toBe(false);
    expect(mainFullGatePasses(context, null)).toBe(false);
  });

  it.each(MAIN_FULL_JOBS)('cannot omit or skip any complete obligation: %s', (name) => {
    const needs = completeNeeds();
    delete needs[name];
    expect(mainFullGatePasses(context, needs)).toBe(false);
    for (const result of ['skipped', 'failure', 'cancelled', 'timed_out', '']) {
      needs[name] = { result };
      expect(mainFullGatePasses(context, needs), `${name}: ${result}`).toBe(false);
    }
  });

  it('wires main admission and aggregate through injected reads and explicit outputs', () => {
    const env = {
      GITHUB_REPOSITORY: context.repository,
      GITHUB_EVENT_NAME: context.eventName,
      GITHUB_REF: context.ref,
      GITHUB_SHA: context.sha,
    };
    const stdout = capture();
    const routes: string[] = [];
    runCiScope(['main-admit'], {
      cwd: '/unused',
      env,
      stdout,
      stderr: capture(),
      request: (route) => {
        routes.push(route);
        return { object: { sha: 'b'.repeat(40) } };
      },
    });
    expect(stdout.text()).toBe('admitted=false\nverify=false\nmacos=false\n');
    expect(routes).toEqual(['git/ref/heads/main']);
    const passed = capture();
    runCiScope(['gate-main'], {
      cwd: '/unused',
      env: { ...env, MAIN_FULL_NEEDS: JSON.stringify(completeNeeds()) },
      stdout: passed,
      stderr: capture(),
      request: () => {
        throw new Error('must not read');
      },
    });
    expect(passed.text()).toContain(`succeeded at ${context.sha}`);
    expect(() =>
      runCiScope(['gate-main'], {
        cwd: '/unused',
        env: { ...env, MAIN_FULL_NEEDS: '{}' },
        stdout: capture(),
        stderr: capture(),
      })
    ).toThrow('missing, unexpected or unsuccessful obligations');
  });

  it('posts one bounded failure notification with real job IDs, and never retries a write', () => {
    const calls: { route: string; method?: string; body?: string }[] = [];
    const request = (route: string, method?: string, body?: string) => {
      calls.push({ route, method, body });
      if (route.startsWith('actions/'))
        return { total_count: 2, jobs: [job(7, 'failure'), job(8)] };
      if (method === 'POST') return { id: 99 };
      return [];
    };
    expect(notifyMainFullFailure(context, request)).toEqual({ created: true, commentId: 99 });
    expect(calls.map(({ route }) => route)).toEqual([
      'actions/runs/42/attempts/1/jobs?per_page=100&page=1',
      'issues/3054/comments?per_page=100&page=1',
      'issues/3054/comments',
    ]);
    const post = calls[2];
    expect(post.method).toBe('POST');
    expect(post.body).toContain(`failed at ${context.sha}, attempt 1`);
    expect(post.body).toContain('[job 7](https://github.com/pj-tmt/tmt/actions/runs/42/job/7)');
    expect(post.body).not.toContain('job 8');
    let writes = 0;
    const uncertain = new Error('POST response lost');
    expect(() =>
      notifyMainFullFailure(context, (route, method) => {
        if (method === 'POST') {
          writes += 1;
          throw uncertain;
        }
        return request(route, method);
      })
    ).toThrow(uncertain);
    expect(writes).toBe(1);
  });

  it('retains a prior bot notification but does not trust an operator-controlled marker', () => {
    for (const login of ['github-actions[bot]', 'someone']) {
      let writes = 0;
      const result = notifyMainFullFailure(context, (route, method) => {
        if (route.startsWith('actions/')) return { total_count: 1, jobs: [job(7, 'failure')] };
        if (method === 'POST') {
          writes += 1;
          return { id: 99 };
        }
        return [{ id: 77, user: { login }, body: '<!-- main-full:42:1 -->' }];
      });
      expect(result).toEqual(
        login === 'github-actions[bot]'
          ? { created: false, commentId: 77 }
          : { created: true, commentId: 99 }
      );
      expect(writes).toBe(login === 'github-actions[bot]' ? 0 : 1);
    }
  });

  it('finishes bounded pagination before notifying and refuses incomplete or oversized evidence', () => {
    const routes: string[] = [];
    notifyMainFullFailure(context, (route, method) => {
      routes.push(route);
      if (route.startsWith('actions/'))
        return {
          total_count: 101,
          jobs: route.endsWith('page=1')
            ? Array.from({ length: 100 }, (_, i) => job(i + 1))
            : [job(101, 'failure')],
        };
      return method === 'POST' ? { id: 99 } : [];
    });
    expect(routes).toHaveLength(4);
    expect(routes[1]).toContain('page=2');
    for (const response of [
      { total_count: 1, jobs: [job(1)] },
      { total_count: 2, jobs: [job(1, 'failure')] },
      { total_count: 501, jobs: [] },
    ]) {
      let writes = 0;
      expect(() =>
        notifyMainFullFailure(context, (_route, method) => {
          if (method === 'POST') writes += 1;
          return response;
        })
      ).toThrow();
      expect(writes).toBe(0);
    }
  });

  it('keeps every full worker in the aggregate and only the notifier can write', () => {
    const workflow = readFileSync(
      new URL('../../../.github/workflows/ci.yml', import.meta.url),
      'utf8'
    );
    const job = (name: string) =>
      workflow.split(`\n  ${name}:\n`)[1].split(/\n {2}[a-z0-9-]+:\n/)[0];
    const ids = [...workflow.split('\njobs:\n')[1].matchAll(/^  ([a-z][a-z0-9-]+):\n/gm)].map(
      (match) => match[1]
    );
    expect(ids.sort()).toEqual(
      [...MAIN_FULL_JOBS, 'release-rehearsal', 'complete-main', 'notify-main-failure'].sort()
    );
    const complete = job('complete-main');
    const needs = complete.split('    needs:\n')[1].split('    if:')[0];
    expect([...needs.matchAll(/^      - (.+)$/gm)].map((match) => match[1]).sort()).toEqual(
      [...MAIN_FULL_JOBS].sort()
    );
    expect(complete).toContain('always()');
    expect(complete).toContain("github.event_name == 'push'");
    expect(complete).toContain("github.ref == 'refs/heads/main'");
    expect(complete).toContain("needs.changes.outputs.main_full_admitted == 'true'");
    expect(complete).toContain('MAIN_FULL_NEEDS: ${{ toJSON(needs) }}');
    expect(complete).toContain('ci-scope.mjs gate-main');
    expect(workflow.split('\npermissions:\n')[1].split('\nconcurrency:')[0].trim()).toBe(
      'contents: read'
    );
    for (const name of ids.filter((name) => name !== 'notify-main-failure')) {
      expect(job(name), name).not.toMatch(
        /^      (?:issues|contents|actions|pull-requests): write$/m
      );
    }
    const notify = job('notify-main-failure');
    expect(notify).toContain('needs: [changes, complete-main]');
    expect(notify).toContain(
      "needs.changes.result == 'failure' || needs.complete-main.result == 'failure'"
    );
    expect(notify).toContain("github.repository == 'pj-tmt/tmt'");
    expect(notify).toContain("github.event_name == 'push'");
    expect(notify).toContain("github.ref == 'refs/heads/main'");
    expect(notify).toContain('issues: write');
    expect(notify).toContain('GH_TOKEN: ${{ github.token }}');
    expect(notify).toContain('persist-credentials: false');
    expect(notify).toContain('ci-scope.mjs notify-main-failure');
    expect(workflow).toContain('name: Code quality');
    expect(workflow).toContain('name: Unit tests');
    expect(workflow).toContain('name: Docker E2E\n');
    expect(workflow).toContain('name: Native package matrix');
  });

  it('admits all main pushes without cancelling useful work or falling back to seed', () => {
    const workflow = readFileSync(
      new URL('../../../.github/workflows/ci.yml', import.meta.url),
      'utf8'
    );
    const push = workflow.split('  push:\n')[1].split('  schedule:')[0];
    expect(push).toContain('branches: [main]');
    expect(push).not.toContain('paths:');
    expect(workflow).toContain(
      "'ci-main-verification' || format('ci-{0}', github.event.pull_request.number || github.ref)"
    );
    expect(workflow).toContain("cancel-in-progress: ${{ github.event_name == 'pull_request' }}");
    expect(workflow).toContain(
      "if: steps.event.outputs.verify != 'true' && github.event_name != 'push'"
    );
    expect(workflow).toContain("if: steps.main.outputs.admitted == 'true'");
    expect(workflow).toContain('ci-scope.mjs main-admit >> "$GITHUB_OUTPUT"');
    expect(workflow).toContain('ci-scope.mjs main-full >> "$GITHUB_OUTPUT"');
    for (const name of ['Check conventional PR titles', 'Check current PR Agent attribution']) {
      const step = workflow.split(`      - name: ${name}\n`)[1].split('\n      - ')[0];
      expect(step).toContain(
        "if: github.event_name == 'pull_request' || github.event_name == 'merge_group'"
      );
    }
    const block = (name: string) =>
      workflow.split(`\n  ${name}:\n`)[1].split(/\n {2}[a-z0-9-]+:\n/)[0];
    const targets = (name: string) =>
      [...block(name).matchAll(/target: ([a-z0-9_-]+)/g)].map((match) => match[1]).sort();
    expect(targets('native-runtime-build')).toEqual([
      'aarch64-unknown-linux-musl',
      'x86_64-unknown-linux-musl',
    ]);
    expect(targets('native-runtime-build-macos')).toEqual([
      'aarch64-apple-darwin',
      'x86_64-apple-darwin',
    ]);
    expect(targets('packed-native-install')).toEqual([
      'aarch64-unknown-linux-musl',
      'aarch64-unknown-linux-musl',
      'x86_64-unknown-linux-musl',
      'x86_64-unknown-linux-musl',
    ]);
    expect(targets('packed-native-install-macos')).toEqual([
      'aarch64-apple-darwin',
      'x86_64-apple-darwin',
    ]);
  });
});

describe('native dependency notices selection', () => {
  it.each([
    'rust/Cargo.lock',
    'rust/rust-toolchain.toml',
    'rust/about.toml',
    'rust/about.hbs',
    'rust/licenses/yrs-0.28.0/LICENSE',
    'rust/Cargo.toml',
    'extensions/tmt-colab/rust/tmt-colab/Cargo.toml',
    'drivers/herdr/rust/tmt-driver-herdr/Cargo.toml',
    'dist-workspace.toml',
    '.github/components.json',
    '.github/workflows/ci.yml',
    'scripts/build-native-artifact.sh',
    'typescript/scripts/verify-native-notices.mjs',
    'typescript/scripts/native-artifact-policy.mjs',
    'typescript/scripts/packed-command.mjs',
  ])('selects notice generation for %s', (file) => {
    expect(selectNativeNotices([file])).toBe(true);
  });

  it.each([
    'rust/Cargo.lock.backup',
    'rust/MyCargo.toml',
    'rust/about.toml.backup',
    'rust/licenses-other/LICENSE',
    'extensions/tmt-colab/rust/tmt-colab/Cargo.toml.backup',
    'rust/crates/tmt-core/src/request.rs',
    'typescript/scripts/verify-native-notices.mjs.backup',
    'site/src/chapters/start.mdx',
    'DEVELOPMENT.md',
    'typescript/pnpm-lock.yaml',
  ])('does not select unrelated or prefix-similar %s', (file) => {
    expect(selectNativeNotices([file])).toBe(false);
  });

  it('retains notice verification on an empty conservative diff', () => {
    expect(selectNativeNotices([])).toBe(true);
  });

  it('routes the selected job result into the native gate without delaying Code quality', () => {
    const ci = readFileSync(new URL('../../../.github/workflows/ci.yml', import.meta.url), 'utf8');
    const job = (name: string) => {
      const block = ci.split(`\n  ${name}:\n`)[1];
      expect(block).toBeDefined();
      return block.split(/\n {2}[a-z0-9-]+:\n/)[0];
    };
    const quality = job('code-quality');
    const native = job('native-install-gate');
    const notices = job('native-notices');
    expect(quality).toContain('needs: changes');
    expect(quality).not.toContain('native-notices');
    expect(native).toMatch(/needs:[\s\S]*native-notices,/);
    expect(native).toContain('NOTICES_SELECTED: ${{ needs.changes.outputs.native_notices }}');
    expect(native).toContain('NOTICES_RESULT: ${{ needs.native-notices.result }}');
    expect(native).toContain('gate "$NOTICES_SELECTED" "$NOTICES_RESULT"');
    expect(notices).toContain(
      "needs.changes.outputs.verify == 'true' && needs.changes.outputs.native_notices == 'true'"
    );
    expect(notices).toContain('uses: actions/cache/restore@v4');
    expect(notices).toContain('save-if: false');
    expect(notices).toContain('cargo fetch --locked');
    expect(notices).toContain('cargo-about --version 0.9.2 --features cli --locked');
    expect(notices).toContain('node typescript/scripts/verify-native-notices.mjs');
    expect(notices).not.toMatch(
      /cargo-dist|cargo build|dist build|pnpm|corepack|setup-tooling|continue-on-error|: write|environment:/
    );
    expect(ciGatePasses('true', ['success'])).toBe(true);
    for (const result of ['skipped', 'failure', 'cancelled', ''])
      expect(ciGatePasses('true', [result])).toBe(false);
    expect(ciGatePasses('false', ['skipped'])).toBe(true);
    expect(ciGatePasses('', ['skipped'])).toBe(false);
  });

  it('gates the Colab chrome spec through the native gate with a required fixture', () => {
    const ci = readFileSync(new URL('../../../.github/workflows/ci.yml', import.meta.url), 'utf8');
    const job = (name: string) => {
      const block = ci.split(`\n  ${name}:\n`)[1];
      expect(block).toBeDefined();
      return block.split(/\n {2}[a-z0-9-]+:\n/)[0];
    };
    const chrome = job('colab-chrome');
    const native = job('native-install-gate');
    expect(chrome).toContain('needs: changes');
    expect(chrome).toContain(
      "needs.changes.outputs.verify == 'true' && needs.changes.outputs.colab_chrome == 'true'"
    );
    expect(chrome).toContain('permissions:\n      contents: read');
    expect(chrome).toContain('persist-credentials: false');
    expect(chrome).toContain('shared-key: native-rust\n          save-if: false');
    expect(chrome).toContain('-p tmt-colab --test socket --no-run --message-format=json');
    expect(chrome).toContain(
      'key: colab-chrome-chromium-${{ runner.os }}-${{ runner.arch }}-${{ steps.playwright.outputs.version }}'
    );
    expect(chrome).toContain('uses: actions/cache/restore@v4');
    expect(chrome).toContain(
      "if: github.ref == 'refs/heads/main' && steps.browsers.outputs.cache-hit != 'true'"
    );
    expect(chrome).toContain('retention-days: 7');
    // Chromium's system packages go through the bounded apt action, never `--with-deps`' own
    // unbounded apt-get update. The list is Playwright's `ubuntu24.04-x64` tools + chromium set for
    // the pinned version; a version bump must refresh the list here and in the job together.
    const playwright = JSON.parse(
      readFileSync(
        new URL('../../../extensions/tmt-colab/typescript/app/package.json', import.meta.url),
        'utf8'
      )
    ).devDependencies['@playwright/test'];
    expect(playwright).toBe('1.63.0');
    expect(chrome).not.toContain('--with-deps');
    expect(chrome).toContain('runs-on: ubuntu-24.04');
    expect(chrome).toContain('timeout-minutes: 10');
    const packages = [
      ...(
        chrome.match(
          /uses: \.\/\.github\/actions\/apt-install\n {8}with:\n {10}packages: >-\n((?: {12}.+\n)+)/
        )?.[1] ?? ''
      )
        .split(/\s+/)
        .filter(Boolean),
    ];
    expect(packages).toEqual([
      ...['xvfb', 'fonts-noto-color-emoji', 'fonts-unifont', 'libfontconfig1', 'libfreetype6'],
      ...['xfonts-cyrillic', 'xfonts-scalable', 'fonts-liberation', 'fonts-ipafont-gothic'],
      ...['fonts-wqy-zenhei', 'fonts-tlwg-loma-otf', 'fonts-freefont-ttf'],
      ...['libasound2t64', 'libatk-bridge2.0-0t64', 'libatk1.0-0t64', 'libatspi2.0-0t64'],
      ...['libcairo2', 'libcups2t64', 'libdbus-1-3', 'libdrm2', 'libgbm1', 'libglib2.0-0t64'],
      ...['libnspr4', 'libnss3', 'libpango-1.0-0', 'libx11-6', 'libxcb1', 'libxcomposite1'],
      ...['libxdamage1', 'libxext6', 'libxfixes3', 'libxkbcommon0', 'libxrandr2'],
    ]);
    expect(chrome).toContain(
      'run: pnpm --filter @tmt/colab-app exec playwright install chromium\n'
    );
    expect(chrome.indexOf('actions/apt-install')).toBeLessThan(
      chrome.indexOf('playwright install chromium')
    );
    expect(chrome).toContain("COLAB_CHROME_REQUIRED: '1'");
    expect(chrome).toContain(
      'COLAB_CHROME_FIXTURE_EXECUTABLE: ${{ steps.fixture.outputs.executable }}'
    );
    expect(chrome).toContain('e2e/chrome.spec.ts --list --reporter=json');
    expect(chrome).toContain('e2e/chrome.spec.ts --workers=1 --reporter=list,json');
    expect(chrome.trim()).not.toContain('continue-on-error');
    expect(chrome).not.toMatch(/--retries|sleep /);
    expect(chrome.indexOf('sh scripts/install-ci-rust.sh ')).toBeLessThan(
      chrome.indexOf('uses: Swatinem/rust-cache@')
    );
    expect(ci).toContain(
      "      colab_chrome: ${{ steps.mainfull.outputs.colab_chrome || steps.scope.outputs.colab_chrome || steps.queue.outputs.colab_chrome || 'false' }}"
    );
    expect(native).toMatch(/needs:[\s\S]*colab-chrome,/);
    expect(native).toContain('CHROME_SELECTED: ${{ needs.changes.outputs.colab_chrome }}');
    expect(native).toContain('CHROME_RESULT: ${{ needs.colab-chrome.result }}');
    expect(native).toContain('gate "$CHROME_SELECTED" "$CHROME_RESULT"');
    expect(job('code-quality')).not.toContain('colab-chrome');
    expect(job('complete-main')).toContain('      - colab-chrome\n');
    expect(MAIN_FULL_JOBS).toContain('colab-chrome');
    const spec = readFileSync(
      new URL('../../../extensions/tmt-colab/typescript/app/e2e/chrome.spec.ts', import.meta.url),
      'utf8'
    );
    expect(spec).toContain("process.env.COLAB_CHROME_REQUIRED === '1'");
  });
});

describe('CI area selection', () => {
  it('attributes Digest releases to its own component while retaining full native checks', () => {
    const paths = [
      'extensions/tmt-digest/rust/tmt-digest/Cargo.toml',
      'extensions/tmt-digest/rust/tmt-digest/src/main.rs',
      'extensions/tmt-digest/rust/tmt-digest/tests/cli.rs',
    ];
    const map = parseComponentMap(
      readFileSync(new URL('../../../.github/components.json', import.meta.url), 'utf8')
    );
    expect(isReleased(map, 'digest')).toBe(true);
    for (const file of paths) {
      expect(ownerOf(file, map)).toBe('digest');
      expect(releasedComponentsForPath(file, map).map(({ name }) => name)).toEqual(['digest']);
      expect(selectNativeScope([file], map)).toBe('full');
      expect(selectCiAreas([file], map)).toEqual({
        native: true,
      });
    }
    expect(ownerOf('extensions/tmt-digest-other/src/main.rs', map)).toBe('cli');
  });

  it('keeps the extension state leaf privately owned by Remote with full native verification', () => {
    const paths = [
      'rust/crates/tmt-extension-state/Cargo.toml',
      'rust/crates/tmt-extension-state/src/lib.rs',
      'rust/crates/tmt-extension-state/tests/state.rs',
    ];
    for (const file of paths) expect(ownerOf(file)).toBe('tmt-remote');
    expect(
      isReleased(
        parseComponentMap(
          readFileSync(new URL('../../../.github/components.json', import.meta.url), 'utf8')
        ),
        'tmt-remote'
      )
    ).toBe(true);
    const rows = explainCiSelection(paths);
    for (const row of rows) {
      expect(row.owner).toBe('tmt-remote');
      expect(row.rule).toBe('native-source');
      expect(selectCiAreas([row.path])).toEqual(
        selectCiAreas(['rust/crates/tmt-invoke/src/lib.rs'])
      );
    }
    expect(ownerOf('rust/crates/tmt-extension-state-other/src/lib.rs')).toBe('cli');
  });

  it('runs the full native workspace for Colab contract vectors that Remote mirrors', () => {
    const vectors = [
      'extensions/tmt-colab/contracts/vectors/deploy-declaration-v1.json',
      'extensions/tmt-colab/contracts/vectors/deploy-declaration-v1.plan.json',
    ];
    for (const row of explainCiSelection(vectors)) {
      expect(row.owner).toBe('tmt-colab');
      expect(row.rule).toBe('unmapped');
      expect(row.native).toBe(true);
    }
    expect(selectNativeScope(vectors)).toBe('full');
    const ci = readFileSync(new URL('../../../.github/workflows/ci.yml', import.meta.url), 'utf8');
    expect(ci).toContain('uses: ./.github/workflows/workspace-partitions.yml');
    const workspace = readFileSync(
      new URL('../../../.github/workflows/workspace-partitions.yml', import.meta.url),
      'utf8'
    );
    expect(workspace).toContain('cargo test --locked --workspace --doc');
    expect(workspace).toContain('workspace-nextest.mjs inventory');
  });

  it('keeps the extension serve leaf selected by its consumers with full native verification', () => {
    const paths = [
      'rust/crates/tmt-extension-serve/Cargo.toml',
      'rust/crates/tmt-extension-serve/src/lib.rs',
      'rust/crates/tmt-extension-serve/tests/handoff.rs',
    ];
    for (const file of paths) expect(ownerOf(file)).toBe('extension-serve');
    for (const row of explainCiSelection(paths)) {
      expect(row.owner).toBe('extension-serve');
      expect(row.rule).toBe('native-source');
      expect(selectCiAreas([row.path])).toEqual(
        selectCiAreas(['rust/crates/tmt-invoke/src/lib.rs'])
      );
    }
    expect(ownerOf('rust/crates/tmt-extension-serve-other/src/lib.rs')).toBe('cli');
  });

  it('keeps the extension objects leaf privately owned by Remote with full native verification', () => {
    const paths = [
      'rust/crates/tmt-extension-objects/Cargo.toml',
      'rust/crates/tmt-extension-objects/src/lib.rs',
      'rust/crates/tmt-extension-objects/src/codec/tests.rs',
    ];
    for (const row of explainCiSelection(paths)) {
      expect(row.owner).toBe('tmt-remote');
      expect(row.rule).toBe('native-source');
      expect(selectCiAreas([row.path])).toEqual(
        selectCiAreas(['rust/crates/tmt-invoke/src/lib.rs'])
      );
    }
    expect(ownerOf('rust/crates/tmt-extension-objects-other/src/lib.rs')).toBe('cli');
  });

  it('rejects a misspelled private-release declaration', () => {
    expect(() =>
      parseComponentMap(
        JSON.stringify({ components: { addon: { owns: ['addon'], release: 'false' } } })
      )
    ).toThrow('release must be boolean');
  });

  it('owns colab-client privately while frozen Office stays unselected', () => {
    const file = 'extensions/tmt-colab/typescript/colab-client/src/object.ts';
    expect(ownerOf(file)).toBe('colab-client');
    const map = parseComponentMap(
      readFileSync(new URL('../../../.github/components.json', import.meta.url), 'utf8')
    );
    expect(isReleased(map, 'colab-client')).toBe(false);
    expect(selectCiAreas([file])).toEqual({ native: true });
    const workflow = readFileSync(
      new URL('../../../.github/workflows/ci.yml', import.meta.url),
      'utf8'
    );
    expect(workflow).toContain(
      'pnpm --filter @tmt/colab-client install --frozen-lockfile --ignore-scripts'
    );
    for (const command of ['check', 'test'])
      expect(workflow).toContain(`pnpm --filter @tmt/colab-client --fail-if-no-match ${command}`);
  });

  it('owns the private colab app and checks it without selecting frozen Office', () => {
    const file = 'extensions/tmt-colab/typescript/app/src/renderer.ts';
    expect(ownerOf(file)).toBe('colab-app');
    const map = parseComponentMap(
      readFileSync(new URL('../../../.github/components.json', import.meta.url), 'utf8')
    );
    expect(isReleased(map, 'colab-app')).toBe(false);
    expect(selectCiAreas([file])).toEqual({ native: false });
    const workflow = readFileSync(
      new URL('../../../.github/workflows/ci.yml', import.meta.url),
      'utf8'
    );
    expect(workflow).toContain(
      'pnpm --filter @tmt/colab-app install --frozen-lockfile --ignore-scripts'
    );
    for (const command of ['check', 'test', 'build'])
      expect(workflow).toContain(`pnpm --filter @tmt/colab-app --fail-if-no-match ${command}`);
  });

  it('verifies the browser presentation leaf without narrowing native or activating Office', () => {
    for (const file of [
      'design/browser-ui/package.json',
      'design/browser-ui/src/static.ts',
      'design/browser-ui/src/header.tsx',
      'design/browser-ui/scripts/generate-static-css.mjs',
      'design/browser-ui/generated/static.css',
    ]) {
      expect(ownerOf(file)).toBe('browser-ui');
      expect(selectCiAreas([file])).toEqual({ native: true });
      expect(selectNativeScope([file])).toBe('full');
    }
    const ci = readFileSync(new URL('../../../.github/workflows/ci.yml', import.meta.url), 'utf8');
    const quality = ci.split('\n  code-quality:\n')[1].split(/\n {2}[a-z0-9-]+:\n/)[0];
    const leaf = quality
      .split('      - name: Verify isolated browser presentation leaf\n')[1]
      .split('      - name:')[0];
    expect(leaf).toContain('working-directory: typescript');
    expect(leaf).toContain(
      'pnpm --filter @tmt/browser-ui install --frozen-lockfile --ignore-scripts'
    );
    for (const command of ['check', 'test'])
      expect(leaf).toContain(`pnpm --filter @tmt/browser-ui --fail-if-no-match ${command}`);
    expect(leaf).toContain('test/tooling/browser-ui-boundary.test.ts');
    expect(leaf).not.toMatch(/continue-on-error|\n\s+if:/);
    const unit = ci.split('\n  unit-tests:\n')[1].split('\n  docker-e2e-shard-1:\n')[0];
    const install = 'pnpm --filter @tmt/browser-ui install --frozen-lockfile --ignore-scripts';
    const admitsGuardInputs = (text: string): boolean => {
      const step = text
        .split('      - name: Install locked browser presentation guard inputs\n')[1]
        ?.split('      - name:')[0];
      const installAt = text.indexOf(install);
      const testsAt = text.indexOf('pnpm test:run');
      return (
        !!step?.includes('working-directory: typescript') &&
        step.includes(`run: ${install}`) &&
        !/continue-on-error|\n\s+if:/.test(step) &&
        installAt >= 0 &&
        testsAt > installAt
      );
    };
    expect(admitsGuardInputs(unit)).toBe(true);
    const withoutInstall = unit.replace(install, 'pnpm --filter tmt install --frozen-lockfile');
    expect(admitsGuardInputs(withoutInstall)).toBe(false);
    const installStep =
      '      - name: Install locked browser presentation guard inputs\n' +
      unit
        .split('      - name: Install locked browser presentation guard inputs\n')[1]
        .split('      - name:')[0];
    expect(admitsGuardInputs(unit.replace(installStep, '') + installStep)).toBe(false);
  });

  it('selects the add-on workflow only for shell/tool inputs without narrowing look-alikes', () => {
    expect(selectCiAreas(['extensions/tmt-remote/typescript/browser-addon/src/popup.ts'])).toEqual({
      native: false,
    });
    for (const file of [
      'extensions/tmt-remote/typescript/browser-addon-other/src/popup.ts',
      'extensions/tmt-remote/rust/tmt-remote/src/main.rs',
      '.github/workflows/browser-addon.yml.orig',
    ]) {
      expect(selectCiAreas([file])).toMatchObject({ native: true });
    }
    expect(ownerOf('extensions/tmt-remote/typescript/browser-addon/src/popup.ts')).toBe(
      'browser-addon'
    );
    const workflow = readFileSync(
      new URL('../../../.github/workflows/browser-addon.yml', import.meta.url),
      'utf8'
    );
    expect(workflow).toMatch(/^on:\n  pull_request:\n    paths:/m);
    expect(workflow).toMatch(/^  workflow_dispatch:/m);
    expect(workflow).not.toContain('continue-on-error');
    const filters = [...workflow.matchAll(/^      - '([^']+)'$/gm)].map((m) => globToRegExp(m[1]));
    const selected = (file: string) => filters.some((filter) => filter.test(file));
    for (const file of [
      'extensions/tmt-remote/typescript/browser-addon/src/popup.ts',
      '.github/workflows/browser-addon.yml',
      '.github/actions/setup-tooling/action.yml',
      'scripts/retry-command.sh',
      'typescript/package.json',
      'typescript/pnpm-workspace.yaml',
      'typescript/pnpm-lock.yaml',
    ])
      expect(selected(file), file).toBe(true);
    for (const file of [
      'rust/crates/tmt-core/src/lib.rs',
      'extensions/tmt-remote/typescript/browser-addon-other/src/popup.ts',
      '.github/workflows/browser-addon.yml.orig',
      'ARCHITECTURE.md',
    ])
      expect(selected(file), file).toBe(false);
    expect(workflow).toContain('--fail-if-no-match');
    expect(workflow).toContain('test:browser');
  });

  it('selects native checks for shared design tokens without narrowing look-alikes', () => {
    const map = parseComponentMap(
      readFileSync(new URL('../../../.github/components.json', import.meta.url), 'utf8')
    );
    for (const name of ['tokens.json', 'tokens-plugin.ts', 'package.json']) {
      const file = `design/tokens/${name}`;
      expect(explainCiSelection([file])[0].rule).toBe('design-tokens');
      expect([...releasedComponentNamesOfPath(file, map)].sort()).toEqual([
        'cli',
        'tmt-colab',
        'tmt-remote',
      ]);
      expect(selectCiAreas([file])).toEqual({ native: true });
      expect(selectNativeScope([file])).toBe('full');
    }
    const lookalike = 'design/tokens-other/tokens.json';
    expect(explainCiSelection([lookalike])[0].rule).toBe('unmapped');
    expect(selectCiAreas([lookalike])).toEqual({
      native: true,
    });
  });

  it.each(['colab-header.css', 'reader-style.css', 'notice-card.css'])(
    'retains full native verification for embedded Colab %s',
    (name) => {
      const file = `extensions/tmt-colab/typescript/app/src/${name}`;
      expect(explainCiSelection([file])[0].rule).toBe('colab-native-chrome');
      expect(selectCiAreas([file])).toEqual({ native: true });
      expect(selectNativeScope([file])).toBe('full');
      expect(explainCiSelection([`${file}.backup`])[0].rule).toBe('unmapped');
    }
  );

  it.each([
    'rust/Cargo.toml',
    'rust/Cargo.lock',
    'rust/rust-toolchain.toml',
    'rust/crates/tmt-core/src/room.rs',
    'rust/crates/tmt-adapters/src/api.rs',
    'rust/crates/tmt-adapters/src/setup/providers.rs',
    'rust/crates/tmt-cli/src/office_facade.rs',
    'rust/crates/tmt-cli/src/api_command.rs',
    'rust/crates/tmt-cli/src/extension_install_command.rs',
    'skills/tmt/SKILL.md',
    'typescript/package.json',
    'typescript/pnpm-lock.yaml',
    'typescript/pnpm-workspace.yaml',
    '.github/workflows/ci.yml',
    '.github/components.json',
    'typescript/scripts/ci-scope.mjs',
    'typescript/test/support/cli-process.ts',
    'typescript/test/e2e/harness.ts',
    'typescript/test/e2e/Dockerfile',
    'typescript/test/native/api.test.ts',
    'typescript/test/tooling/ci-scope.test.ts',
    'typescript/test/e2e/ops.e2e.test.ts',
    'new-owner/file.ts',
    'extensions/tmt-ops-other/file.rs',
    'typescript/apps/office/src/main.tsx',
    'contracts/office/request.json',
    'docs/office.md.orig',
    'rust/archive/NATIVE-INSTALL.md',
  ])('keeps native verification without frozen Office work for %s', (file) => {
    expect(selectCiAreas([file])).toEqual({ native: true });
  });

  it.each([
    'extensions/tmt-office/rust/tmt-office/src/main.rs',
    'extensions/tmt-office/contracts/request.json',
    'extensions/tmt-office/skills/tmt-office/SKILL.md',
    'typescript/test/native/office-board.test.ts',
    'typescript/test/e2e/office-command.e2e.test.ts',
    'typescript/test/stress/office-native-installation-capacity.test.ts',
    'typescript/test/support/office-world.ts',
    'extensions/tmt-office/typescript/services/office/firestore.rules',
    'extensions/tmt-office/typescript/apps/office-other/file.ts',
    'extensions/tmt-office/rusty/file.rs',
    '.github/workflows/office-browser.yml',
    '.dockerignore',
    'typescript/scripts/verify-office-emulators.mjs',
  ])('retires Office product checks while retaining native inputs for %s', (file) => {
    expect(selectCiAreas([file])).toEqual({ native: true });
  });

  it.each([
    'ARCHITECTURE.md',
    'DEVELOPMENT.md',
    'contracts/extension-api.md',
    '.agents/skills/tmt-dev/SKILL.md',
    '.github/pull_request_template.md',
    'site/src/chapters/ops.mdx',
    'site/pnpm-lock.yaml',
    'site/src/blog/build.mjs',
    'site/src/posts/agent-conversations/article.html',
    '.github/workflows/site.yml',
    'extensions/tmt-remote/typescript/browser-addon/src/popup.ts',
  ])('selects nothing beyond Code quality for %s', (file) => {
    expect(selectCiAreas([file])).toEqual({ native: false });
  });

  it('keeps empty diffs conservative for native work without inventing Office ownership', () => {
    expect(selectCiAreas([])).toEqual({ native: true });
  });

  it('unions ownership across mixed paths and both sides of a no-renames diff', () => {
    expect(
      selectCiAreas(['extensions/tmt-office/typescript/apps/office/removed.ts', 'rust/new.rs'])
    ).toEqual({ native: true });
    expect(selectCiAreas(['ARCHITECTURE.md', 'typescript/package.json'])).toEqual({
      native: true,
    });
  });
});

describe('component map', () => {
  const repository = fileURLToPath(new URL('../../../', import.meta.url));
  const mapText = readFileSync(path.join(repository, '.github/components.json'), 'utf8');
  const map = parseComponentMap(mapText);
  const tracked = (): string[] =>
    runPackedCommand('git', ['ls-files', '-z'], { cwd: repository, env: process.env })
      .split('\0')
      .filter(Boolean);
  const invalid = (change: (value: Record<string, any>) => void) => {
    const value = JSON.parse(mapText);
    change(value);
    return () => parseComponentMap(JSON.stringify(value));
  };

  it('admits the canonical two-generator map and retains single-generator maps', () => {
    expect(
      map.components
        .find((component) => component.name === 'tmt-colab')!
        .generatedInputs.map((entry) => entry.variable)
    ).toEqual(['TMT_COLAB_APP_DIR', 'TMT_COLAB_HOSTING_DIR']);
    const single = JSON.parse(mapText);
    single.components['tmt-colab'].generatedInputs.pop();
    expect(
      parseComponentMap(JSON.stringify(single)).components.find(
        (component) => component.name === 'tmt-colab'
      )!.generatedInputs
    ).toHaveLength(1);
  });

  it.each(['reversed', 'variable', 'generator', 'includeSite', 'third'])(
    'rejects %s generated-input drift',
    (drift) => {
      expect(
        invalid((value) => {
          const entries = value.components['tmt-colab'].generatedInputs;
          if (drift === 'reversed') entries.reverse();
          else if (drift === 'third')
            entries.push({
              ...entries[1],
              variable: 'TMT_COLAB_Z_DIR',
              generator: 'extensions/extra/build.rs',
              includeSite: 'extensions/extra/src.rs',
            });
          else entries[1][drift] = entries[0][drift];
        })
      ).toThrow('permits at most two canonical generators in ascending variable order');
    }
  );

  it('every workflow Node version-file input names a tracked repository file', () => {
    const files = tracked();
    for (const workflow of files.filter((file) =>
      /^\.github\/workflows\/[^/]+\.ya?ml$/.test(file)
    )) {
      const text = readFileSync(path.join(repository, workflow), 'utf8');
      for (const match of text.matchAll(/^\s*node-version-file:\s*([^\n]+)$/gm)) {
        const versionFile = match[1].trim().replace(/^['"]|['"]$/g, '');
        expect(files, `${workflow}: node-version-file ${versionFile} is not tracked`).toContain(
          versionFile
        );
      }
    }
  });

  it('says which components are released: all but the ones that declare release: false', () => {
    expect(isReleased(map, 'cli')).toBe(true);
    expect(isReleased(map, 'ops')).toBe(true);
    expect(() => isReleased(map, 'squad')).toThrow('Unknown component squad.');
    // Office is parked, as the private browser add-on is.
    expect(() => isReleased(map, 'office')).toThrow('Unknown component office.');
    expect(isReleased(map, 'browser-addon')).toBe(false);
    const parked = parseComponentMap(
      JSON.stringify({ components: { office: { owns: ['office'], release: false } } })
    );
    expect(isReleased(parked, 'office')).toBe(false);
    expect(() => isReleased(map, 'nothing')).toThrow('Unknown component nothing.');
  });

  it.each([
    ['rust/crates/tmt-cli-style', ['cli']],
    ['rust/crates/tmt-cli-style/src/lib.rs', ['cli']],
    ['rust/crates/tmt-invoke/src/lib.rs', ['cli']],
    ['rust/crates/tmt-tui/src/lib.rs', []],
    ['rust/crates/tmt-tui-other/src/lib.rs', ['cli']],
    ['extensions/tmt-ops/rust/src/lib.rs', ['ops']],
    ['extensions/tmt-ops-other/rust/src/lib.rs', ['cli']],
    ['extensions/tmt-office/src/lib.rs', ['cli']],
    ['extensions/tmt-colab/rust/src/lib.rs', ['tmt-colab']],
    ['rust/crates/tmt-test-support/src/lib.rs', []],
    ['typescript/test/native/ops.test.ts', ['cli']],
  ])('finds released root membership independently of CI ownership for %s', (file, names) => {
    expect(releasedComponentsForPath(file, map).map((component) => component.name)).toEqual(names);
    expect(releasedComponentsForPath(file).map((component) => component.name)).toEqual(names);
  });

  it('matches globs by whole path, with ** across directories and newlines', () => {
    expect(globToRegExp('rust/**').test('rust/crates/tmt-core/src/lib.rs')).toBe(true);
    expect(globToRegExp('rust/**').test('rustic/lib.rs')).toBe(false);
    expect(globToRegExp('apps/office/**').test('apps/office/name with\nnewline.ts')).toBe(true);
    expect(globToRegExp('docs/*.md').test('docs/guide.md')).toBe(true);
    expect(globToRegExp('docs/*.md').test('docs/nested/guide.md')).toBe(false);
    expect(globToRegExp('docs/*.md').test('docs/guide.mdx')).toBe(false);
    expect(globToRegExp('a?c').test('abc')).toBe(true);
    expect(globToRegExp('a?c').test('a/c')).toBe(false);
    expect(globToRegExp('a.b+c').test('aXb+c')).toBe(false);
  });

  it('rejects a malformed map instead of selecting the wrong work', () => {
    expect(invalid((value) => (value.rules[0].consumers = ['native', 'browser']))).toThrow(
      'unknown consumer'
    );
    expect(invalid((value) => (value.rules[1].id = value.rules[0].id))).toThrow('repeated');
    expect(invalid((value) => (value.rules[0].paths = []))).toThrow('non-empty list');
    expect(invalid((value) => delete value.rules[0].why)).toThrow('needs a reason');
    expect(invalid((value) => (value.components = {}))).toThrow('no components');
    expect(invalid((value) => (value.components.cli.owns = 'rust'))).toThrow('non-empty list');
  });

  it('resolves one owner per path: a selectedBy glob wins, look-alike prefixes do not match', () => {
    const owner = (file: string) => ownerOf(file, map);
    expect(owner('rust/crates/tmt-core/src/lib.rs')).toBe('cli');
    expect(owner('typescript/test/native/api.test.ts')).toBe('cli');
    expect(owner('extensions/tmt-office/rust/tmt-office/src/main.rs')).toBe('cli');
    expect(owner('extensions/tmt-ops/rust/tmt-ops/src/main.rs')).toBe('ops');
    expect(owner('typescript/test/native/office-board.test.ts')).toBe('cli');
    expect(owner('typescript/test/e2e/ops.e2e.test.ts')).toBe('ops');
    expect(owner('typescript/test/e2e/ops-reminder.e2e.test.ts')).toBe('ops');
    expect(owner('extensions/tmt-ops-other/file.rs')).toBe('cli');
    expect(owner('extensions/tmt-officer/file.rs')).toBe('cli');
  });

  it('explains every changed path with its owner, rule and consumers', () => {
    const rows = explainCiSelection(
      ['ARCHITECTURE.md', 'extensions/tmt-ops/rust/tmt-ops/src/main.rs', 'new-owner/file.ts'],
      map
    );
    expect(rows.map(({ path: file, owner, rule }) => [file, owner, rule])).toEqual([
      ['ARCHITECTURE.md', 'cli', 'prose'],
      ['extensions/tmt-ops/rust/tmt-ops/src/main.rs', 'ops', 'native-source'],
      ['new-owner/file.ts', 'cli', 'unmapped'],
    ]);
    expect(rows[0]).toMatchObject({ native: false });
    expect(rows[2]).toMatchObject({ native: true });
  });

  it('renders the evidence table with the map digest, and bounds a large diff', () => {
    const rows = explainCiSelection(['ARCHITECTURE.md', 'rust/lib.rs'], map);
    const text = renderSelectionEvidence({
      base: 'a'.repeat(40),
      head: 'b'.repeat(40),
      rows,
      areas: selectCiAreas(['ARCHITECTURE.md', 'rust/lib.rs'], map),
      digest: map.digest,
    });
    expect(text).toContain('Diff `aaaaaaaaaaaa...bbbbbbbbbbbb`, 2 changed path(s)');
    expect(text).toContain(`sha256:${map.digest}`);
    expect(text).toContain('Selected: native=true.');
    expect(text).toContain('| `ARCHITECTURE.md` | cli | prose | nothing |');
    expect(text).toContain('| `rust/lib.rs` | cli | native-source | native |');
    const many = explainCiSelection(
      Array.from({ length: 130 }, (_, index) => `.agents/file-${index}.md`),
      map
    );
    const bounded = renderSelectionEvidence({
      base: 'a'.repeat(40),
      head: 'b'.repeat(40),
      rows: many,
      areas: { native: false },
      digest: map.digest,
    });
    expect(bounded.match(/^\| `\.agents\//gm)).toHaveLength(100);
    expect(bounded).toContain('30 more path(s) not listed: prose 30.');
  });

  it.each([
    [['extensions/tmt-ops/rust/tmt-ops/src/main.rs'], 'ops'],
    [['extensions/tmt-ops/skills/tmt-ops/SKILL.md', 'ARCHITECTURE.md'], 'ops'],
    [['typescript/test/e2e/ops.e2e.test.ts', 'typescript/test/native/ops.test.ts'], 'ops'],
    [['typescript/test/e2e/ops-reminder.e2e.test.ts'], 'ops'],
    [['extensions/tmt-ops/rust/tmt-ops/src/main.rs', 'rust/Cargo.lock'], 'full'],
    [
      ['extensions/tmt-ops/rust/tmt-ops/src/main.rs', 'rust/crates/tmt-cli-style/src/lib.rs'],
      'full',
    ],
    [['extensions/tmt-ops/rust/tmt-ops/src/main.rs', 'design/cli-style.md'], 'full'],
    [['extensions/tmt-ops/rust/tmt-ops/src/main.rs', 'typescript/test/native/api.test.ts'], 'full'],
    [['extensions/tmt-ops/rust/tmt-ops/src/main.rs', 'new-owner/file.ts'], 'full'],
    [['extensions/tmt-ops-other/file.rs'], 'full'],
    [['rust/crates/tmt-core/src/lib.rs'], 'full'],
    [['rust/crates/tmt-cli-style/src/lib.rs'], 'full'],
    [['rust/crates/tmt-invoke/src/lib.rs'], 'full'],
    [['extensions/tmt-office/rust/tmt-office/src/main.rs'], 'full'],
    [['ARCHITECTURE.md', 'DEVELOPMENT.md'], 'none'],
    [['extensions/tmt-office/typescript/apps/office/src/main.tsx'], 'full'],
    [[], 'full'],
  ] as const)('selects the native scope for %j: %s', (paths, scope) => {
    expect(selectNativeScope([...paths], map)).toBe(scope);
  });

  it('names the checks a scoped component runs and none for the other scopes', () => {
    expect(scopedChecks('ops', map)).toEqual({
      nativeTests: [
        'ops.test.ts',
        'extension-install.test.ts',
        'extension-upgrade-proof.test.ts',
        'uninstall.test.ts',
      ],
      e2eFiles: ['ops.e2e.test.ts', 'ops-reminder.e2e.test.ts'],
    });
    expect(scopedChecks('full', map)).toEqual({ nativeTests: [], e2eFiles: [] });
    expect(scopedChecks('none', map)).toEqual({ nativeTests: [], e2eFiles: [] });
    expect(scopedChecks('office', map)).toEqual({ nativeTests: [], e2eFiles: [] });
  });

  it('rejects scoped checks that are not plain file names', () => {
    for (const bad of ['../ops.test.ts', 'a b.test.ts', 'ops.test.ts;rm', '$HOME']) {
      expect(invalid((value) => (value.components.ops.scopedChecks.nativeTests = [bad]))).toThrow(
        'plain file names'
      );
    }
    expect(invalid((value) => (value.components.ops.scopedChecks.e2eFiles = []))).toThrow(
      'non-empty list'
    );
    expect(invalid((value) => delete value.components.ops.scopedChecks.nativeTests)).toThrow(
      'non-empty list'
    );
  });

  it('lists every test that runs the Ops binary or reads Ops, so the scoped run cannot miss one', () => {
    const files = tracked();
    const checks = scopedChecks('ops', map);
    for (const file of checks.nativeTests) {
      expect(files, file).toContain(`typescript/test/native/${file}`);
    }
    for (const file of checks.e2eFiles) {
      expect(files, file).toContain(`typescript/test/e2e/${file}`);
    }
    // Tests that name Ops without exercising its executable or sources.
    const namesOnly: Record<string, string> = {
      'typescript/test/e2e/driver-consent.e2e.test.ts':
        'host-driver protocol ops operation names, no Ops executable or source',
      'typescript/test/e2e/nested-host-hooks.e2e.test.ts':
        'host-driver protocol ops operation names, no Ops executable or source',
      'typescript/test/native/api.test.ts':
        'a synthetic owned skill named tmt-ops; no Ops executable or source',
      'typescript/test/native/office-freeze.test.ts':
        'the remaining installable catalog name after Office removal, without Ops execution or sources',
      'typescript/test/native/setup-guided.test.ts': 'the install hint text',
    };
    const mention = /\bops\b|tmt-ops/i;
    const undeclared = files.filter(
      (file) =>
        /^typescript\/test\/(native\/[^/]+\.test\.ts|e2e\/[^/]+\.e2e\.test\.ts)$/.test(file) &&
        mention.test(readFileSync(path.join(repository, file), 'utf8')) &&
        !checks.nativeTests.includes(path.basename(file)) &&
        !checks.e2eFiles.includes(path.basename(file)) &&
        !(file in namesOnly)
    );
    expect(
      undeclared,
      'these tests mention Ops: add them to scopedChecks or to the names-only list'
    ).toEqual([]);
  });

  it('keeps the tooling tests, which the Ops scope skips, from reading Ops sources', () => {
    const files = tracked().filter((file) =>
      /^typescript\/test\/(tooling|support)\/[^/]+\.ts$/.test(file)
    );
    // Code quality runs these on every pull request, so skipping Unit tests does not skip
    // them: they check inputs that Ops-only changes can also change.
    const alwaysRun = [
      'typescript/test/tooling/markdown-links.test.ts',
      'typescript/test/tooling/ci-scope.test.ts',
      'typescript/test/tooling/component-skills.test.ts',
      'typescript/test/tooling/release-cut.test.ts',
      'typescript/test/tooling/release-cut-live.test.ts',
      'typescript/test/tooling/release-rehearsal.test.ts',
      'typescript/test/tooling/release-version-injection.test.ts',
      'typescript/test/tooling/release-workflow.test.ts',
    ];
    const qualityCommand = /vp test run ([^\n]+)/.exec(
      readFileSync(path.join(repository, '.github/workflows/ci.yml'), 'utf8')
    )?.[1];
    for (const file of alwaysRun) {
      expect(qualityCommand, file).toContain(file.replace('typescript/', ''));
    }
    const readers = files.filter(
      (file) =>
        !alwaysRun.includes(file) &&
        /extensions\/tmt-ops|rust\/target\/debug\/tmt-ops/.test(
          readFileSync(path.join(repository, file), 'utf8')
        )
    );
    // support/native-artifact.ts packages the built Ops binary for the native tests
    // (not run by Unit tests); it is not a tooling test.
    expect(readers.filter((file) => file.startsWith('typescript/test/tooling/'))).toEqual([]);
  });

  it('every rule and selectedBy glob still matches a tracked file', () => {
    const files = tracked();
    expect(files.length).toBeGreaterThan(100);
    const globs = [
      ...map.rules.flatMap((rule) => rule.globs.map((glob) => [`rule ${rule.id}`, glob])),
      ...map.components.flatMap((component) =>
        component.selectedBy.map(({ glob }) => [`component ${component.name}`, glob])
      ),
    ];
    for (const [owner, glob] of globs) {
      const pattern = globToRegExp(glob);
      expect(
        files.some((file) => pattern.test(file)),
        `${owner}: ${glob} matches no tracked file`
      ).toBe(true);
    }
    for (const component of map.components) {
      if (isComponentRetired(component.name)) continue;
      for (const root of [...component.owns, ...component.excludes]) {
        expect(
          root === '.' || files.some((file) => file.startsWith(`${root}/`)),
          `${component.name}: root ${root} has no tracked file`
        ).toBe(true);
      }
    }
    for (const file of files) expect(ownerOf(file, map), file).not.toBe('unowned');
  });

  it('keeps prose inert only while no test or build input reads it', () => {
    const files = tracked();
    const inert = explainCiSelection(files, map)
      .filter((row) => row.rule === 'prose')
      .map((row) => row.path);
    expect(inert).toContain('ARCHITECTURE.md');
    // Files that name inert prose without reading it in a job that inert paths would skip:
    // formatter file lists run in Code quality on every change, and the rest only mention
    // a name.
    const mentions: Record<string, string> = {
      'typescript/scripts/format-workspace.mjs':
        'formatter file lists, run by Code quality on every change',
      'typescript/test/tooling/format-workspace.test.ts':
        'selection path fixtures only; never reads prose contents',
      'typescript/test/tooling/ci-scope.test.ts': 'path fixtures for the selector tests',
      'typescript/test/tooling/release-rehearsal.test.ts':
        'path fixtures for the selection tests; never reads prose contents',
      'typescript/test/fixtures/release-cut-history.json':
        'immutable historical path/map data; cut tests compare strings without reading the named prose',
      'scripts/dev-disk-check.sh': 'names DEVELOPMENT.md in a message',
      'typescript/test/tooling/guide-budget.test.ts':
        'counts DEVELOPMENT.md and ARCHITECTURE.md lines; Code quality runs it on every change',
      'typescript/scripts/guide-budgets.json':
        'guide-budget limits, enforced by Code quality on every change',
      'typescript/scripts/infra-tick-summary.mjs':
        'manual Infra report reads guide sizes; CI tests inject contents and never run the live report',
      'typescript/test/tooling/infra-tick-summary.test.ts':
        'injected guide-name fixtures only; never reads the named prose contents',
      'typescript/test/tooling/markdown-links.test.ts':
        'reads inert tracked Markdown link destinations; Code quality runs it on every change',
      'typescript/test/tooling/pr-agent-check.test.ts':
        'reads the preset table of native-release.md; Code quality runs it on every change',
      '.github/components.json': 'the map names the prose in its own rules',
      '.github/repository-layout.json':
        'top-level names only; the layout guard never reads listed prose',
      'rust/crates/tmt-adapters/src/skill_installation/owned_tests.rs':
        'a fixture file name, not the repository README',
      'site/src/chapters/dev-extension.mdx': 'the handbook site links to the contract on GitHub',
    };
    const layout = JSON.parse(
      readFileSync(path.join(repository, '.github/repository-layout.json'), 'utf8')
    ) as { languageExceptions: Record<string, string> };
    // The handbook site links to the contract on GitHub in each listed translation too.
    const translatedDevExtensionMentions = new Set(
      Object.keys(layout.languageExceptions).map((directory) => `${directory}/dev-extension.mdx`)
    );
    // Only files that contain ".md" at all can name prose.
    const candidates = runPackedCommand('git', ['grep', '-l', '-z', '-I', '-F', '.md', '--', '.'], {
      cwd: repository,
      env: process.env,
    })
      .split('\0')
      .filter(
        (file: string) =>
          file &&
          !file.endsWith('.md') &&
          !(file in mentions) &&
          !translatedDevExtensionMentions.has(file)
      );
    expect(candidates.length).toBeGreaterThan(20);
    const readers: string[] = [];
    for (const file of candidates) {
      const text = readFileSync(path.join(repository, file), 'utf8');
      const references = new Set(text.match(/[A-Za-z0-9_./-]+\.md\b/g) ?? []);
      for (const reference of references) {
        if (
          inert.some(
            (prose) =>
              reference === prose ||
              (prose.includes('/')
                ? reference.endsWith(`/${prose}`)
                : /^(\.\.?\/)+/.test(reference) && reference.replace(/^(\.\.?\/)+/, '') === prose)
          )
        ) {
          readers.push(`${file} -> ${reference}`);
        }
      }
    }
    expect(readers, 'these files read or name inert prose; map the prose to its consumer').toEqual(
      []
    );
  });
});

describe('Colab browser-only native scope', () => {
  const app = 'extensions/tmt-colab/typescript/app/';

  it.each([
    'src/style.css',
    'src/reader.ts',
    'src/hosted-reader.ts',
    'test/reader-editor-browser.tsx',
    'e2e/reader-editor.spec.ts',
    'acceptance/hosted-reader.proof.ts',
    'acceptance/harness/firestore-proxy.ts',
  ])('retains the browser consumer without full native work for %s', (relative) => {
    const file = app + relative;
    expect(selectNativeScope([file])).toBe('none');
    expect(selectColabApp([file])).toBe(true);
    expect(selectNativeScope(['README.md', file])).toBe('none');
  });

  it('keeps the generator checked by unconditional app quality/build commands', () => {
    const file = 'extensions/tmt-colab/scripts/generate-renderer-style.mjs';
    expect(selectNativeScope([file])).toBe('none');
    const scripts = JSON.parse(
      readFileSync(new URL('../../../' + app + 'package.json', import.meta.url), 'utf8')
    ).scripts;
    for (const command of ['check', 'build'])
      expect(scripts[command]).toContain('generate-renderer-style.mjs --check');
  });

  it.each([
    'src/colab-header.css',
    'src/reader-style.css',
    'src/notice-card.css',
    'src/app-build.ts',
    'src/main.tsx',
    'src/reader-main.tsx',
    'src/hosted-main.tsx',
    'src/hosted-entry.ts',
    'src/new.config.ts',
    'public/renderer.html',
    'index.html',
    'package.json',
    'vite.config.ts',
    'playwright.hosted.config.ts',
    'acceptance/hosted-bundle.spec.ts',
  ])('keeps native/build/entry/config input %s full', (relative) => {
    expect(selectNativeScope([app + relative])).toBe('full');
  });

  it.each([
    'rust/crates/tmt-core/src/lib.rs',
    'extensions/tmt-colab/rust/tmt-colab/app-entries.txt',
    'extensions/tmt-colab/skills/tmt-colab/SKILL.md',
    'extensions/tmt-colab/skills/tmt-colab/references/editing.md',
    'extensions/tmt-colab/contracts/vectors/authority-v1.json',
    'extensions/tmt-colab/scripts/cloud-sync-emulator.mjs',
    'extensions/tmt-colab/scripts/hosted-reader-emulator.mjs',
    'extensions/tmt-colab/typescript/colab-client/src/link.ts',
    'extensions/tmt-remote/typescript/remote-client/test/vectors.json',
    'typescript/scripts/ci-scope.mjs',
    'typescript/test/e2e/mock-claude-channel.mjs',
    '.github/workflows/ci.yml',
    'extensions/tmt-colab/typescript/app-other/src/reader.ts',
    'extensions/tmt-colab/typescript/app/src-other/reader.ts',
    'extensions/tmt-colab/typescript/app/src/new.input',
    'extensions/tmt-colab/scripts/generate-renderer-style.mjs.backup',
    'unknown/new-input',
  ])('retains full native coverage for %s, alone or mixed with browser input', (file) => {
    expect(selectNativeScope([file])).toBe('full');
    expect(selectNativeScope([app + 'src/style.css', file])).toBe('full');
  });

  it('keeps every tracked hosted-bundle input native as that consumer evolves', () => {
    const root = fileURLToPath(new URL('../../../', import.meta.url));
    const inventory: string = runPackedCommand('git', ['ls-files', '-z'], {
      cwd: root,
      env: process.env,
    });
    const files = inventory.split('\0').filter((file) => file && selectColabHosted([file]));
    expect(files).toContain(app + 'acceptance/hosted-bundle.spec.ts');
    const browserOnly = new Set([
      app + 'src/reader-link.ts',
      app + 'src/reader.ts',
      app + 'e2e/served.spec.ts',
    ]);
    for (const file of files)
      expect(selectNativeScope([file]), file).toBe(browserOnly.has(file) ? 'none' : 'full');
  });
});

it('keeps every tracked Rust embedded Colab app input in full native scope', () => {
  const root = fileURLToPath(new URL('../../../', import.meta.url));
  const inventory: string = runPackedCommand('git', ['ls-files', '-z'], {
    cwd: root,
    env: process.env,
  });
  const tracked = new Set(inventory.split('\0').filter(Boolean));
  const embedded = new Set<string>();
  for (const source of tracked) {
    if (!source.endsWith('.rs')) continue;
    const text = readFileSync(path.join(root, source), 'utf8');
    // Match the literal and manifest-relative concat! forms used by the Docker copy audit.
    const inputs = [...text.matchAll(/include_(?:str|bytes)!\s*\(\s*"([^"\n]+)"/g)].map(
      ([, value]) => path.posix.normalize(path.posix.join(path.posix.dirname(source), value))
    );
    let crate = path.posix.dirname(source);
    while (crate !== '.' && !tracked.has(`${crate}/Cargo.toml`)) crate = path.posix.dirname(crate);
    for (const [, value] of text.matchAll(
      /include_(?:str|bytes)!\s*\(\s*concat!\(\s*env!\("CARGO_MANIFEST_DIR"\),\s*"([^"\n]+)"/g
    ))
      inputs.push(path.posix.normalize(`${crate}${value}`));
    for (const input of inputs) {
      if (!input.startsWith('extensions/tmt-colab/typescript/app/')) continue;
      embedded.add(input);
      expect(selectNativeScope([input]), `${source} embeds ${input}`).toBe('full');
    }
  }
  expect([...embedded]).toContain('extensions/tmt-colab/typescript/app/src/colab-header.css');
});

describe('Colab contract prose native scope', () => {
  const contract = 'extensions/tmt-colab/contracts/colab-v1.md';

  it('scopes only the exact prose path out of native verification', () => {
    expect(selectNativeScope([contract])).toBe('none');
    expect(selectNativeScope(['README.md', contract])).toBe('none');
    expect(explainCiSelection([contract])[0].rule).toBe('colab-contract-prose');
  });

  it.each([
    'extensions/tmt-colab/contracts/vectors/authority-v1.json',
    'extensions/tmt-colab/contracts/vectors/README.md',
    'extensions/tmt-colab/contracts/colab-v1.md.backup',
    'extensions/tmt-colab/contracts/colab-v2.md',
    'extensions/tmt-colab/contracts-other/colab-v1.md',
    'extensions/tmt-colab-other/contracts/colab-v1.md',
    'extensions/tmt-colab/rust/tmt-colab/app-entries.txt',
    'extensions/tmt-colab/skills/tmt-colab/SKILL.md',
    'rust/crates/tmt-core/src/lib.rs',
    'unknown/new-input',
  ])('keeps %s full, alone or mixed with contract prose', (file) => {
    expect(selectNativeScope([file])).toBe('full');
    expect(selectNativeScope([contract, file])).toBe('full');
  });
});

describe('remote Rust retains full CI coverage', () => {
  const remote = 'extensions/tmt-remote/rust/tmt-remote/';
  const all = { native: true };

  it.each(['Cargo.toml', 'src/core.rs', 'tests/door.rs'])(
    'explicitly selects full workspace checks for %s despite private release ownership',
    (suffix) => {
      const files = [remote + suffix];
      expect(explainCiSelection(files)).toMatchObject([
        { owner: 'tmt-remote', rule: 'remote-rust', ...all },
      ]);
      expect(selectCiAreas(files)).toEqual(all);
      expect(selectNativeScope(files)).toBe('full');
      expect(scopedChecks(selectNativeScope(files))).toEqual({ nativeTests: [], e2eFiles: [] });
    }
  );

  it.each([
    'rust/Cargo.toml',
    'rust/Cargo.lock',
    'rust/crates/tmt-cli-style/src/lib.rs',
    'rust/crates/tmt-cli/tests/architecture.rs',
    'extensions/tmt-ops/rust/tmt-ops/src/main.rs',
    'unknown/new-file',
  ])('retains all consumers and full scope with %s', (other) => {
    expect(selectNativeScope([remote + 'src/main.rs', other])).toBe('full');
    expect(selectCiAreas([remote + 'src/main.rs', other])).toEqual(all);
  });

  it.each([
    'extensions/tmt-remote/rust-other/src/main.rs',
    'extensions/tmt-remote-other/rust/src/main.rs',
    'extensions/tmt-remote/typescript/remote-client/src/canonical.ts',
  ])('does not swallow %s into the Rust rule', (file) => {
    expect(explainCiSelection([file])[0].rule).toBe('unmapped');
    expect(selectCiAreas([file])).toEqual(all);
    expect(selectNativeScope([file])).toBe('full');
  });

  it('keeps the browser separate and rejects every missing selected job', () => {
    const browser = 'extensions/tmt-remote/typescript/browser-addon/src/popup.ts';
    expect(explainCiSelection([browser])[0].rule).toBe('browser-addon');
    expect(selectNativeScope([browser])).toBe('none');
    const scope = selectNativeScope([remote + 'tests/door.rs']);
    const results = {
      nativeRust: 'success',
      unitTests: 'success',
      e2eShard1: 'success',
      e2eShard2: 'success',
      runtimeBuild: 'success',
      packedInstall: 'success',
      macosRuntimeBuild: 'success',
      macosPackedInstall: 'success',
    };
    expect(nativeGatePasses(scope, results, 'true')).toBe(true);
    for (const job of Object.keys(results)) {
      for (const result of ['skipped', 'failure', 'cancelled', undefined]) {
        expect(nativeGatePasses(scope, { ...results, [job]: result }, 'true')).toBe(false);
      }
    }
    expect(nativeGatePasses('remote', results, 'true')).toBe(false);
  });

  it.each([
    {
      discovery: 'door_lifecycle: test\n1 test, 0 benchmarks',
      listStatus: 0,
      testStatus: 0,
      status: 0,
    },
    { discovery: '0 tests, 0 benchmarks', listStatus: 0, testStatus: 0, status: 1 },
    { discovery: 'door_lifecycle: test', listStatus: 7, testStatus: 0, status: 7 },
    { discovery: 'door_lifecycle: test', listStatus: 0, testStatus: 8, status: 8 },
  ])('executes producer discovery/compilation with status $status', (fixture) => {
    const workflow = readFileSync(
      new URL('../../../.github/workflows/workspace-partitions.yml', import.meta.url),
      'utf8'
    );
    const step = workflow
      .split('      - name: Test and build workspace\n')[1]
      ?.split('\n      - name:')[0];
    expect(step).toContain("if: inputs.scope == 'full'");
    expect(step).toContain('working-directory: rust');
    const script = step.split('        run: |\n')[1].replace(/^ {10}/gm, '');
    const root = mkdtempSync(path.join(tmpdir(), 'tmt-remote-ci-'));
    try {
      // Execute the committed shell and actual inventory helper with bounded fake Cargo/nextest.
      // Native execution failure is independently guarded by the partition aggregate tests.
      mkdirSync(path.join(root, 'rust'));
      symlinkSync(fileURLToPath(new URL('../..', import.meta.url)), path.join(root, 'typescript'));
      const executable = path.join(root, 'rust', 'fixture-tests');
      writeExecutable(
        executable,
        '#!/bin/sh\ncase "$*" in *--ignored*) echo "ignored: test";; *) printf "a: test\\nb: test\\nignored: test\\n";; esac\n',
        0o755
      );
      const build = JSON.stringify({
        reason: 'compiler-artifact',
        profile: { test: true },
        executable,
      });
      const suites = {
        'rust-suites': {
          fixture: {
            'binary-id': 'fixture',
            'binary-path': executable,
            testcases: { a: { ignored: false }, b: { ignored: false }, ignored: { ignored: true } },
          },
        },
        'rust-build-meta': { 'non-test-binaries': {} },
      };
      writeExecutable(
        path.join(root, 'cargo-nextest'),
        `#!/usr/bin/env node
const inventory = ${JSON.stringify(suites)};
const args = process.argv.slice(2);
const slice = args[args.indexOf('--partition') + 1];
const selected = args.includes('--partition') ? (slice === 'slice:1/2' ? 'a' : 'b') : null;
for (const [name,test] of Object.entries(inventory['rust-suites'].fixture.testcases)) {
  test['filter-match'] = test.ignored ? {status:'mismatch',reason:'ignored'} :
    selected === null || name === selected ? {status:'matches'} : {status:'mismatch',reason:'partition'};
}
console.log(JSON.stringify(inventory));
`,
        0o755
      );
      writeExecutable(
        path.join(root, 'cargo'),
        `#!/bin/sh
printf '%s\\n' "$*" >> "$CALLS"
case "$*" in
  'test --locked -p tmt-remote -- --list') printf '%s\\n' "$DISCOVERY"; exit "$LIST_STATUS" ;;
  'test --locked --workspace --no-run --message-format=json') printf '%s\\n' "$BUILD"; exit "$TEST_STATUS" ;;
esac
`,
        0o755
      );
      runPackedCommand(
        'bash',
        ['--noprofile', '--norc', '-eo', 'pipefail', '-c', `{\n${script}\n} 2>&1`],
        {
          cwd: path.join(root, 'rust'),
          env: {
            ...process.env,
            PATH: `${root}:${process.env.PATH}`,
            CALLS: path.join(root, 'calls'),
            BUILD: build,
            RUNNER_TEMP: path.join(root, 'temp'),
            K: '2',
            DISCOVERY: fixture.discovery,
            LIST_STATUS: String(fixture.listStatus),
            TEST_STATUS: String(fixture.testStatus),
          },
          expectedStatus: fixture.status === 0 ? 0 : 1,
        }
      );
      const calls = readFileSync(path.join(root, 'calls'), 'utf8').trim().split('\n');
      const expected = ['test --locked -p tmt-remote -- --list'];
      if (fixture.status === 0 || fixture.testStatus !== 0)
        expected.push('test --locked --workspace --no-run --message-format=json');
      expect(calls).toEqual(expected);
      const phases = JSON.parse(
        readFileSync(path.join(root, 'temp/workspace-proof/phases.json'), 'utf8')
      );
      if (fixture.listStatus !== 0) expect(phases[0].exit).toBe(fixture.listStatus);
      if (fixture.testStatus !== 0) expect(phases[1].exit).toBe(fixture.testStatus);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});

describe('hosted reader emulator selection', () => {
  it.each([
    'extensions/tmt-colab/typescript/app/src/hosted-reader.ts',
    'extensions/tmt-colab/typescript/app/src/hosted-app.css',
    'extensions/tmt-colab/typescript/app/src/admission.ts',
    'extensions/tmt-colab/typescript/app/src/fold.ts',
    'extensions/tmt-colab/typescript/app/src/fold-protocol.ts',
    'extensions/tmt-colab/typescript/app/src/reader-link.ts',
    'extensions/tmt-colab/typescript/app/src/objects.ts',
    'extensions/tmt-colab/typescript/app/src/catchup.ts',
    'extensions/tmt-colab/typescript/app/src/reader-keys.ts',
    'extensions/tmt-colab/typescript/colab-client/src/link.ts',
    'extensions/tmt-colab/typescript/app/acceptance/hosted-reader.proof.ts',
    'extensions/tmt-colab/typescript/app/playwright.hosted.config.ts',
    'extensions/tmt-colab/scripts/hosted-reader-emulator.mjs',
    'extensions/tmt-colab/scripts/cloud-sync-emulator.mjs',
    'extensions/tmt-colab/rust/tmt-colab/build/hosting.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/object_channel/cloud_sync/append.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/fold.rs',
    'extensions/tmt-colab/contracts/vectors/deploy-declaration-v1.firestore.rules',
    'extensions/tmt-remote/rust/tmt-remote/src/serve.rs',
    'extensions/tmt-remote/rust/tmt-remote/src/object_service/requests.rs',
    'extensions/tmt-remote/rust/tmt-remote/src/firestore_budget.rs',
    'extensions/tmt-remote/rust/tmt-remote/assets/remote-v1.js',
    'extensions/tmt-remote/rust/tmt-remote/assets/settings-v1.js',
    'extensions/tmt-remote/rust/tmt-remote/examples/compose_hosting.rs',
    'extensions/tmt-remote/typescript/remote-client/src/firebase-client.ts',
    'extensions/tmt-remote/typescript/remote-client/src/browser.ts',
    'extensions/tmt-remote/typescript/remote-client/firebase-build-pin.json',
    '.github/workflows/ci.yml',
    '.github/workflows/colab-browser.yml',
    '.github/actions/hosted-reader-emulator/action.yml',
    'typescript/scripts/ci-scope.mjs',
  ])('selects the real producer/reader proof for %s', (file) => {
    expect(selectHostedReaderEmulator([file])).toBe(true);
    expect(selectHostedReaderEmulator(['README.md', file])).toBe(true);
    expect(explainCiSelection([file])[0].colabHostedEmulator).toBe(true);
  });

  it.each([
    'README.md',
    'extensions/tmt-remote/rust/tmt-remote/tests/serve.rs',
    'extensions/tmt-remote/rust/tmt-remote/docs/serve.md',
    'extensions/tmt-remote/rust/tmt-remote/src-other/serve.rs',
    'extensions/tmt-remote/typescript/remote-client/test/emulator/browser.spec.ts',
    'extensions/tmt-remote/typescript/remote-client/test/emulator/colab-source.json',
    'extensions/tmt-remote/typescript/remote-client/firebase-build-pin.json.bak',
    'extensions/tmt-colab/typescript/app/src/page-list.tsx',
    'extensions/tmt-colab/typescript/app/src/hosted.ts',
    'extensions/tmt-colab/typescript/app/acceptance/hosted-bundle.spec.ts',
    'extensions/tmt-colab/typescript/app/acceptance/ask.spec.ts',
    'extensions/tmt-colab/typescript/colab-client/test/link.test.ts',
    '.github/actions/hosted-reader-emulator-other/action.yml',
  ])('does not expand the qualifier to unrelated/test-only input %s', (file) => {
    expect(selectHostedReaderEmulator([file])).toBe(false);
    expect(explainCiSelection([file])[0].colabHostedEmulator).toBe(false);
  });

  it('leaves empty, main and seed browser scope empty', () => {
    expect(selectHostedReaderEmulator([])).toBe(false);
    for (const mode of ['seed', 'full', 'main-full']) {
      let outputs = '';
      runCiScope([mode], {
        cwd: '/unused',
        stdout: { write: (text: string) => (outputs += text) },
        stderr: { write: () => undefined },
      });
      expect(outputs).toContain('colab_hosted_emulator=false\n');
    }
  });

  it('shows the same advisory predicate in path explanations and the evidence table', () => {
    const paths = ['extensions/tmt-colab/typescript/app/src/hosted-reader.ts', 'README.md'];
    const rows = explainCiSelection(paths);
    const text = renderSelectionEvidence({
      base: 'a'.repeat(40),
      head: 'b'.repeat(40),
      rows,
      areas: selectCiAreas(paths),
      digest: 'fixture',
    });
    expect(rows.map((row) => row.colabHostedEmulator)).toEqual([true, false]);
    expect(text).toContain('colab-hosted-emulator');
    expect(text).toContain('| `README.md` |');
  });
});

describe('CI diff and command integration', () => {
  it('reads actual additions, cross-owner renames and deletions with whitespace-safe paths', () => {
    const root = mkdtempSync(path.join(tmpdir(), 'tmt-ci-scope-'));
    const git = (args: string[]): string =>
      runPackedCommand('git', args, {
        cwd: root,
        env: { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null' },
      });
    const commit = (): string => {
      git(['add', '.']);
      git([
        '-c',
        'user.name=TMT Test',
        '-c',
        'user.email=test@example.invalid',
        '-c',
        'commit.gpgsign=false',
        'commit',
        '--quiet',
        '-m',
        'Test fixture\n\nCo-authored-by: Codex <codex@openai.com>',
      ]);
      return git(['rev-parse', 'HEAD']).trim();
    };
    try {
      git(['init', '--quiet']);
      writeFileSync(path.join(root, 'README.md'), 'fixture\n');
      const base = commit();
      mkdirSync(path.join(root, 'extensions/tmt-remote/typescript/browser-addon/docs'), {
        recursive: true,
      });
      const historicalSource = path.join(
        root,
        'extensions/tmt-remote/typescript/browser-addon/docs/name with\nnewline.ts'
      );
      writeFileSync(historicalSource, 'export const fixture = true;\n');
      const historical = commit();
      expect(readChangedCiAreas(base, historical, root)).toEqual({
        native: false,
      });
      mkdirSync(path.join(root, 'extensions/tmt-remote/typescript/browser-addon/app'), {
        recursive: true,
      });
      const source = path.join(
        root,
        'extensions/tmt-remote/typescript/browser-addon/app/name with\nnewline.ts'
      );
      renameSync(historicalSource, source);
      const added = commit();
      expect(readChangedCiAreas(historical, added, root)).toEqual({
        native: false,
      });
      mkdirSync(path.join(root, 'rust'));
      const target = path.join(root, 'rust/fixture.rs');
      renameSync(source, target);
      const moved = commit();
      expect(readChangedCiAreas(added, moved, root)).toEqual({
        native: true,
      });
      rmSync(target);
      const deleted = commit();
      // Removing a core workspace input does not select frozen Office verification.
      expect(readChangedCiAreas(moved, deleted, root)).toEqual({
        native: true,
      });
      expect(() => readChangedCiAreas('--help', deleted, root)).toThrow('exact base and head');
      expect(() => readChangedCiAreas('0'.repeat(40), deleted, root)).toThrow(
        'Packed command failed'
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  }, 5000);

  it('writes only the step outputs to stdout and the evidence to stderr and the summary', () => {
    const root = mkdtempSync(path.join(tmpdir(), 'tmt-ci-scope-run-'));
    const git = (args: string[]): string =>
      runPackedCommand('git', args, {
        cwd: root,
        env: { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null' },
      });
    const commit = (): string => {
      git(['add', '.']);
      git([
        '-c',
        'user.name=TMT Test',
        '-c',
        'user.email=test@example.invalid',
        '-c',
        'commit.gpgsign=false',
        'commit',
        '--quiet',
        '-m',
        'Test fixture',
      ]);
      return git(['rev-parse', 'HEAD']).trim();
    };
    const capture = () => {
      let text = '';
      return { write: (chunk: string) => void (text += chunk), text: () => text };
    };
    try {
      git(['init', '--quiet']);
      writeFileSync(path.join(root, 'ARCHITECTURE.md'), 'fixture\n');
      const base = commit();
      mkdirSync(path.join(root, 'rust'));
      writeFileSync(path.join(root, 'rust/fixture.rs'), '// fixture\n');
      writeFileSync(path.join(root, 'ARCHITECTURE.md'), 'changed\n');
      const head = commit();
      // Outside the fixture repository, which commits everything it finds.
      const summaryFile = `${root}-summary.md`;
      const stdout = capture();
      const stderr = capture();
      runCiScope([base, head], { cwd: root, stdout, stderr, summaryFile });
      const outputs = (text: string) =>
        Object.fromEntries(
          text
            .trim()
            .split('\n')
            .map((line) => [line.slice(0, line.indexOf('=')), line.slice(line.indexOf('=') + 1)])
        );
      const full = outputs(stdout.text());
      expect(full).toMatchObject({
        native: 'true',
        colab_harness: 'false',
        colab_app: 'false',
        colab_hosted: 'false',
        colab_hosted_emulator: 'false',
        native_scope: 'full',
        scoped_native_tests: '',
      });
      // Every retained scenario file is in exactly one of the two shards.
      const shardFiles = [...full.e2e_shard_1.split(' '), ...full.e2e_shard_2.split(' ')];
      expect(shardFiles.sort()).toEqual(listE2eFiles());
      expect(stderr.text()).toContain('### CI selection');
      expect(stderr.text()).toContain('native scope full.');
      expect(stderr.text()).toContain('| `rust/fixture.rs` | cli | native-source | native |');
      expect(stderr.text()).toContain('| `ARCHITECTURE.md` | cli | prose | nothing |');
      expect(readFileSync(summaryFile, 'utf8')).toBe(stderr.text());
      // Without a summary file nothing is written there, and a second run appends.
      const quiet = capture();
      runCiScope([base, head], { cwd: root, stdout: capture(), stderr: quiet });
      expect(quiet.text()).toBe(stderr.text());
      runCiScope([base, head], { cwd: root, stdout: capture(), stderr: capture(), summaryFile });
      expect(readFileSync(summaryFile, 'utf8')).toBe(stderr.text() + stderr.text());
      // An Ops-only change is scoped and names the checks its component runs.
      mkdirSync(path.join(root, 'extensions/tmt-ops/rust/tmt-ops/src'), { recursive: true });
      writeFileSync(path.join(root, 'extensions/tmt-ops/rust/tmt-ops/src/main.rs'), '// x\n');
      const opsHead = commit();
      const scoped = capture();
      const scopedLog = capture();
      runCiScope([head, opsHead], { cwd: root, stdout: scoped, stderr: scopedLog });
      expect(outputs(scoped.text())).toEqual({
        native: 'true',
        colab_harness: 'false',
        colab_app: 'false',
        colab_hosted: 'false',
        colab_hosted_emulator: 'false',
        colab_chrome: 'false',
        native_scope: 'ops',
        native_notices: 'false',
        remote_firestore: 'false',
        colab_firestore: 'false',
        native_sync_emulator: 'false',
        scoped_native_tests:
          'ops.test.ts extension-install.test.ts extension-upgrade-proof.test.ts uninstall.test.ts',
        e2e_shard_1: 'ops.e2e.test.ts ops-reminder.e2e.test.ts',
        e2e_shard_2: '',
      });
      expect(scopedLog.text()).toContain('native scope ops.');
      // A real remote-only git diff must drive the full output consumed by the workflow.
      mkdirSync(path.join(root, 'extensions/tmt-remote/rust/tmt-remote/src'), { recursive: true });
      writeFileSync(path.join(root, 'extensions/tmt-remote/rust/tmt-remote/src/main.rs'), '// x\n');
      const remoteHead = commit();
      const remoteOutput = capture();
      const remoteLog = capture();
      runCiScope([opsHead, remoteHead], { cwd: root, stdout: remoteOutput, stderr: remoteLog });
      expect(outputs(remoteOutput.text())).toEqual({ ...full, colab_hosted_emulator: 'true' });
      expect(remoteLog.text()).toContain(
        'Hosted reader emulator PR selection (hosted reader, native producer or qualifier inputs): true.'
      );
      expect(remoteLog.text()).toContain(
        '| tmt-remote | remote-rust | native, colab-hosted-emulator |'
      );
      // The Rules emulator suite's fixtures select its Unit tests steps and stay a full native scope.
      const rulesDir = 'extensions/tmt-remote/rust/tmt-remote/tests/fixtures/rules';
      mkdirSync(path.join(root, rulesDir), { recursive: true });
      writeFileSync(path.join(root, rulesDir, 'composed.rules'), '// x\n');
      const rulesHead = commit();
      const rulesOutput = capture();
      runCiScope([remoteHead, rulesHead], { cwd: root, stdout: rulesOutput, stderr: capture() });
      expect(outputs(rulesOutput.text())).toEqual({ ...full, remote_firestore: 'true' });
      const colabRulesDir = 'extensions/tmt-colab/firestore';
      mkdirSync(path.join(root, colabRulesDir), { recursive: true });
      writeFileSync(path.join(root, colabRulesDir, 'admission.rules'), '// Colab Rules\n');
      const colabRulesHead = commit();
      const colabRulesOutput = capture();
      const colabRulesLog = capture();
      runCiScope([rulesHead, colabRulesHead], {
        cwd: root,
        stdout: colabRulesOutput,
        stderr: colabRulesLog,
      });
      expect(outputs(colabRulesOutput.text())).toEqual({ ...full, colab_firestore: 'true' });
      expect(colabRulesLog.text()).toContain(
        'Colab Firestore Rules emulator selection (Unit tests job): true.'
      );

      const officePath = path.join(root, 'extensions/tmt-office/docs/fixture.md');
      mkdirSync(path.dirname(officePath), { recursive: true });
      writeFileSync(officePath, 'Office fixture\n');
      const officeHead = commit();
      const browserSelected = (baseHead: string, nextHead: string) => {
        const output = capture();
        runCiScope([baseHead, nextHead], { cwd: root, stdout: output, stderr: capture() });
        return Object.hasOwn(outputs(output.text()), 'office_browser');
      };
      expect(browserSelected(remoteHead, officeHead)).toBe(false);
      mkdirSync(path.join(root, 'docs'), { recursive: true });
      renameSync(officePath, path.join(root, 'docs/moved.md'));
      const movedHead = commit();
      expect(browserSelected(officeHead, movedHead)).toBe(false);
      writeFileSync(officePath, 'Office fixture\n');
      const restoredHead = commit();
      rmSync(officePath);
      const deletedHead = commit();
      expect(browserSelected(restoredHead, deletedHead)).toBe(false);

      // Additions, both rename endpoints and deletions drive the advisory output.
      const colabPath = path.join(root, 'extensions/tmt-colab/contracts/vectors/fixture.json');
      mkdirSync(path.dirname(colabPath), { recursive: true });
      writeFileSync(colabPath, '{}\n');
      const colabHead = commit();
      const colabSelected = (baseHead: string, nextHead: string) => {
        const output = capture();
        runCiScope([baseHead, nextHead], { cwd: root, stdout: output, stderr: capture() });
        return outputs(output.text()).colab_harness;
      };
      expect(colabSelected(deletedHead, colabHead)).toBe('true');
      const movedColab = path.join(root, 'docs/colab-fixture.json');
      renameSync(colabPath, movedColab);
      const movedColabHead = commit();
      expect(colabSelected(colabHead, movedColabHead)).toBe('true');
      renameSync(movedColab, colabPath);
      const restoredColabHead = commit();
      expect(colabSelected(movedColabHead, restoredColabHead)).toBe('true');
      rmSync(colabPath);
      const deletedColabHead = commit();
      expect(colabSelected(restoredColabHead, deletedColabHead)).toBe('true');
      writeFileSync(path.join(root, 'docs/unrelated.md'), 'Unrelated\n');
      const unrelatedHead = commit();
      expect(colabSelected(deletedColabHead, unrelatedHead)).toBe('false');

      expect(() => runCiScope(['only-one'], { cwd: root, stdout, stderr })).toThrow(
        'exact base and head'
      );
      runCiScope(['gate', 'true', 'success'], { cwd: root, stdout, stderr });
      expect(() => runCiScope(['gate', 'true', 'failure'], { cwd: root, stdout, stderr })).toThrow(
        'did not complete successfully'
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
      rmSync(`${root}-summary.md`, { force: true });
    }
  }, 5000);

  it.each([
    'extensions/tmt-colab/typescript/colab-client/src/index.ts',
    'extensions/tmt-colab/typescript/colab-client/test/differential.mjs',
    'extensions/tmt-colab/typescript/colab-client/package.json',
    'extensions/tmt-colab/rust/tmt-colab-model/src/lib.rs',
    'extensions/tmt-colab/rust/tmt-colab-model/examples/browser_authority.rs',
    'extensions/tmt-colab/contracts/vectors/ed25519-829.jsonl',
    '.github/workflows/colab-browser.yml',
    'rust/Cargo.lock',
    'rust/Cargo.toml',
    'typescript/pnpm-lock.yaml',
  ])('selects the advisory Colab harness for %s', (file) => {
    expect(selectColabHarness([file])).toBe(true);
    expect(selectColabHarness(['DEVELOPMENT.md', file])).toBe(true);
  });

  it.each([
    'extensions/tmt-colab/typescript/colab-client-other/src/index.ts',
    'extensions/tmt-colab/rust/tmt-colab-model-other/src/lib.rs',
    'extensions/tmt-colab/contracts/vectors-other/model.json',
    'extensions/tmt-colab/rust/tmt-colab/src/main.rs',
    'extensions/tmt-colab/contracts/colab-v1.md',
    'extensions/tmt-remote/typescript/remote-client/src/index.ts',
    'extensions/tmt-office/typescript/apps/office/src/main.tsx',
    'typescript/scripts/ci-scope.mjs',
    'typescript/pnpm-lock.yaml.backup',
    'rust/Cargo.lock.backup',
    'rust/Cargo.toml.backup',
    '.github/workflows/ci.yml',
    'DEVELOPMENT.md',
    'unknown/input',
  ])('does not select advisory Colab work for %s', (file) => {
    expect(selectColabHarness([file])).toBe(false);
  });

  it.each([
    'extensions/tmt-colab/typescript/app/src/annotation-input.tsx',
    'extensions/tmt-colab/typescript/app/e2e/fold.spec.ts',
    'extensions/tmt-colab/typescript/app/vite.config.ts',
    'extensions/tmt-colab/typescript/colab-client/src/index.ts',
    'design/browser-ui/src/static.css',
    '.github/workflows/colab-browser.yml',
    'typescript/pnpm-lock.yaml',
  ])('selects the advisory Colab app component suite for %s', (file) => {
    expect(selectColabApp([file])).toBe(true);
    expect(selectColabApp(['DEVELOPMENT.md', file])).toBe(true);
  });

  it.each([
    'extensions/tmt-colab/typescript/app-other/src/index.ts',
    'extensions/tmt-colab/rust/tmt-colab/src/main.rs',
    'extensions/tmt-colab/rust/tmt-colab-model/src/lib.rs',
    'extensions/tmt-colab/contracts/colab-v1.md',
    'extensions/tmt-remote/typescript/remote-client/src/index.ts',
    'typescript/pnpm-lock.yaml.backup',
    '.github/workflows/ci.yml',
    'DEVELOPMENT.md',
    'unknown/input',
  ])('does not select the advisory Colab app component suite for %s', (file) => {
    expect(selectColabApp([file])).toBe(false);
  });

  it.each([
    '.github/workflows/colab-browser.yml',
    'extensions/tmt-remote/typescript/remote-client/firebase-build-pin.json',
    'extensions/tmt-remote/typescript/remote-client/vite.firebase.config.ts',
    'extensions/tmt-colab/rust/tmt-colab/build/hosting.rs',
    'extensions/tmt-colab/typescript/app/package.json',
    'extensions/tmt-colab/typescript/app/vite.config.ts',
    'extensions/tmt-colab/typescript/app/acceptance/hosted-bundle.spec.ts',
    'extensions/tmt-colab/rust/tmt-colab/src/serve.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/socket.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/door.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/assets.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/readers.rs',
    'extensions/tmt-colab/rust/tmt-colab/build.rs',
    'extensions/tmt-colab/typescript/app/public/reader.html',
    'extensions/tmt-colab/typescript/app/src/reader-link.ts',
    'extensions/tmt-colab/typescript/app/src/reader-main.tsx',
    'extensions/tmt-colab/typescript/app/src/reader.ts',
    'extensions/tmt-colab/typescript/app/e2e/served.spec.ts',
  ])('selects the served and hosted job for %s', (file) => {
    expect(selectColabHosted([file])).toBe(true);
    expect(selectColabHosted(['DEVELOPMENT.md', file])).toBe(true);
  });

  // The pin manifest is the drift signal: Remote source or package edits do not select the job.
  it.each([
    'extensions/tmt-remote/typescript/remote-client/src/firebase-client.ts',
    'extensions/tmt-remote/typescript/remote-client/package.json',
    'extensions/tmt-remote/typescript/remote-client/firebase-build-pin.json.bak',
    'extensions/tmt-colab/rust/tmt-colab/src/serve_release.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/socket_tests.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/readers/tests.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/object_channel/cloud_sync/reader.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/sync.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/deploy_declaration.rs',
    'extensions/tmt-colab/typescript/app/src/index.tsx',
    'extensions/tmt-colab/typescript/app/acceptance/page-save.spec.ts',
    '.github/workflows/ci.yml',
    'DEVELOPMENT.md',
  ])('does not select the hosted bundle spec for %s', (file) => {
    expect(selectColabHosted([file])).toBe(false);
  });

  it('does not select the hosted bundle spec for an empty diff', () => {
    expect(selectColabHosted([])).toBe(false);
  });

  // The chrome fixture mounts the real router and pages, so any app input selects the gating job,
  // alongside the four native files that render or capture the fallback chrome.
  it.each([
    'extensions/tmt-colab/rust/tmt-colab/src/chrome.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/socket.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/assets.rs',
    'extensions/tmt-colab/rust/tmt-colab/tests/socket.rs',
    'extensions/tmt-colab/typescript/app/e2e/chrome.spec.ts',
    'extensions/tmt-colab/typescript/app/src/colab-header.tsx',
    'extensions/tmt-colab/typescript/app/src/home.tsx',
    'design/browser-ui/src/list.tsx',
    'typescript/pnpm-lock.yaml',
    '.github/workflows/ci.yml',
    '.github/workflows/colab-browser.yml',
  ])('selects the gating Colab chrome spec for %s', (file) => {
    expect(selectColabChrome([file])).toBe(true);
    expect(selectColabChrome(['DEVELOPMENT.md', file])).toBe(true);
  });

  it.each([
    'extensions/tmt-colab/rust/tmt-colab/src/sync.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/serve_release.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/deploy_declaration.rs',
    'extensions/tmt-colab/rust/tmt-colab/tests/state.rs',
    'extensions/tmt-remote/typescript/remote-client/src/firebase-client.ts',
    'DEVELOPMENT.md',
    'unknown/input',
  ])('does not select the Colab chrome spec for %s', (file) => {
    expect(selectColabChrome([file])).toBe(false);
  });

  it('does not select the Colab chrome spec for an empty diff', () => {
    expect(selectColabChrome([])).toBe(false);
  });

  it('requires mapped ownership and does not expand empty Colab app scope', () => {
    expect(selectColabApp([])).toBe(false);
    const map = parseComponentMap(
      JSON.stringify({ components: { cli: { owns: ['.'] } }, rules: [] })
    );
    expect(selectColabApp(['extensions/tmt-colab/typescript/app/src/index.ts'], map)).toBe(false);
  });

  it.each([
    'extensions/tmt-remote/rust/tmt-remote/tests/emulator/suite.mjs',
    'extensions/tmt-remote/rust/tmt-remote/tests/emulator/firebase.json',
    'extensions/tmt-remote/rust/tmt-remote/tests/emulator/Dockerfile',
    'extensions/tmt-remote/rust/tmt-remote/tests/fixtures/rules/composed.rules',
    'extensions/tmt-remote/rust/tmt-remote/tests/fixtures/rules/hostile/path-literal.rules',
    'extensions/tmt-remote/rust/tmt-remote/src/rules.rs',
    'extensions/tmt-remote/rust/tmt-remote/assets/remote-v1.js',
    '.github/workflows/ci.yml',
  ])('selects the Remote Firestore Rules emulator steps for %s', (file) => {
    expect(selectRemoteFirestore([file])).toBe(true);
    expect(selectRemoteFirestore(['DEVELOPMENT.md', file])).toBe(true);
    // The steps live in the Unit tests job, which runs only for a full native scope: a selected
    // path must never be scoped away, or the selected steps would be skipped unexpectedly.
    expect(selectNativeScope([file])).toBe('full');
  });

  it.each([
    'extensions/tmt-remote/rust/tmt-remote/tests/emulator-other/suite.mjs',
    'extensions/tmt-remote/rust/tmt-remote/tests/fixtures/declarations/colab-firestore.json',
    'extensions/tmt-remote/rust/tmt-remote/tests/fixtures/rules-other/x.rules',
    'extensions/tmt-remote/rust/tmt-remote/src/deploy_plan.rs',
    'extensions/tmt-remote/rust/tmt-remote/assets/settings-v1.js',
    'extensions/tmt-remote/rust/tmt-remote/tests/rules.rs',
    '.github/workflows/colab-browser.yml',
    'DEVELOPMENT.md',
    'unknown/input',
  ])('does not select the Remote Firestore Rules emulator steps for %s', (file) => {
    expect(selectRemoteFirestore([file])).toBe(false);
  });

  it('does not select the Rules emulator steps for an empty diff', () => {
    expect(selectRemoteFirestore([])).toBe(false);
  });
  it('runs the Rules emulator as pinned, fail-closed steps of the Unit tests job', () => {
    const read = (file: string) =>
      readFileSync(new URL(`../../../${file}`, import.meta.url), 'utf8');
    const ci = read('.github/workflows/ci.yml');
    const job = ci.slice(ci.indexOf('\n  unit-tests:'), ci.indexOf('\n  # The Docker E2E suite'));
    const steps = job.slice(
      job.indexOf('# Firestore Rules emulator suites'),
      job.indexOf('- name: Run retained developer tooling tests')
    );
    expect(ci).toContain(
      "      remote_firestore: ${{ steps.mainfull.outputs.remote_firestore || steps.scope.outputs.remote_firestore || steps.queue.outputs.remote_firestore || 'false' }}"
    );
    expect(
      steps.match(/^        if: needs\.changes\.outputs\.remote_firestore == 'true'$/gm)
    ).toHaveLength(3);
    expect(
      steps.match(/^        if: needs\.changes\.outputs\.colab_firestore == 'true'$/gm)
    ).toHaveLength(1);
    expect(steps).toContain('timeout-minutes: 8');
    expect(steps).toContain('test -n "$JAVA_HOME_21_X64"');
    expect(steps).toContain('"$JAVA_HOME_21_X64/bin/java" -version');
    // No third-party action, retry, cache or relaxed failure in the selected steps.
    for (const forbidden of ['uses:', 'continue-on-error', 'retry', 'cache', '|| true']) {
      expect(steps).not.toContain(forbidden);
    }
    // Both retained emulator suites use the same pinned tools.
    const pin = 'firebase-tools@15.29.0';
    for (const file of [
      'extensions/tmt-remote/rust/tmt-remote/tests/emulator/Dockerfile',
      'extensions/tmt-colab/rust/tmt-colab/tests/emulator/Dockerfile',
    ]) {
      expect(read(file)).toContain(pin);
    }
    expect(steps).toContain(pin);
    const javaImage = /eclipse-temurin:21-jre-jammy@sha256:[0-9a-f]{64}/;
    expect(
      read('extensions/tmt-remote/rust/tmt-remote/tests/emulator/Dockerfile').match(javaImage)?.[0]
    ).toBe(
      read('extensions/tmt-colab/rust/tmt-colab/tests/emulator/Dockerfile').match(javaImage)?.[0]
    );
  });

  it.each([
    'extensions/tmt-colab/firestore/admission.rules',
    'extensions/tmt-colab/firestore/declaration.json',
    'extensions/tmt-colab/contracts/vectors/deploy-declaration-v1.json',
    'extensions/tmt-colab/contracts/vectors/deploy-declaration-v1.firestore.rules',
    'extensions/tmt-colab/contracts/vectors/deploy-declaration-reference.py',
    'extensions/tmt-colab/rust/tmt-colab/tests/emulator/suite.mjs',
    'extensions/tmt-colab/rust/tmt-colab/tests/emulator/firebase.json',
    'extensions/tmt-colab/rust/tmt-colab/tests/emulator/Dockerfile',
    '.github/workflows/ci.yml',
  ])('selects the Colab Firestore Rules emulator steps for %s', (file) => {
    expect(selectColabFirestore([file])).toBe(true);
    expect(selectColabFirestore(['DEVELOPMENT.md', file])).toBe(true);
    expect(selectNativeScope([file])).toBe('full');
  });

  it.each([
    'extensions/tmt-colab/firestore-other/admission.rules',
    'extensions/tmt-colab/contracts/vectors/other.json',
    'extensions/tmt-colab/contracts/vectors/deploy-declaration.json',
    'extensions/tmt-colab/rust/tmt-colab/tests/emulator-other/suite.mjs',
    'extensions/tmt-colab/rust/tmt-colab/src/main.rs',
    'extensions/tmt-colab/typescript/colab-client/src/index.ts',
    'extensions/tmt-remote/rust/tmt-remote/src/rules.rs',
    'DEVELOPMENT.md',
    'unknown/input',
  ])('does not select the Colab Firestore Rules emulator steps for %s', (file) => {
    expect(selectColabFirestore([file])).toBe(false);
  });

  it('keeps Colab emulator execution selected, pinned and fail closed in Unit tests', () => {
    expect(selectColabFirestore([])).toBe(false);
    const read = (file: string) =>
      readFileSync(new URL(`../../../${file}`, import.meta.url), 'utf8');
    const ci = read('.github/workflows/ci.yml');
    const job = ci.slice(ci.indexOf('\n  unit-tests:'), ci.indexOf('\n  # The Docker E2E suite'));
    expect(ci).toContain(
      "      colab_firestore: ${{ steps.mainfull.outputs.colab_firestore || steps.scope.outputs.colab_firestore || steps.queue.outputs.colab_firestore || 'false' }}"
    );
    expect(job).toContain(
      "if: needs.changes.outputs.verify == 'true' && needs.changes.outputs.native_scope == 'full'"
    );
    expect(job).toContain('timeout-minutes: 15');
    const steps = job.slice(
      job.indexOf('# Firestore Rules emulator suites'),
      job.indexOf('- name: Run retained developer tooling tests')
    );
    expect(steps.match(/npm install --global firebase-tools@15\.29\.0/g)).toHaveLength(1);
    const colab = steps.slice(
      steps.indexOf('- name: Run the Colab Firestore Rules emulator suite')
    );
    expect(colab).toContain("if: needs.changes.outputs.colab_firestore == 'true'");
    expect(colab).toContain('timeout-minutes: 8');
    expect(colab).toContain('firebase emulators:exec --only firestore --project demo-tmt-colab');
    expect(colab).toContain(
      '--config /c/rust/tmt-colab/tests/emulator/firebase.json --non-interactive'
    );
    expect(colab).toContain("'node --test /c/rust/tmt-colab/tests/emulator/suite.mjs'");
    expect(colab).toContain(
      'docker build --tag "$image" extensions/tmt-colab/rust/tmt-colab/tests/emulator'
    );
    expect(colab).toContain('docker run --rm --init --network none');
    expect(colab).toContain('--volume "$GITHUB_WORKSPACE/extensions/tmt-colab:/c:ro" "$image"');
    expect(colab).toContain('trap cleanup EXIT');
    expect(colab).toContain('docker image rm "$image"');
    expect(colab.match(/--volume /g)).toHaveLength(1);
    expect(colab).not.toMatch(/--env|--mount|--network host|--privileged|secrets\./);
    expect(steps).not.toMatch(/continue-on-error|\|\| true|secrets\./);
    expect(read('.github/actions/setup-tooling/action.yml')).toContain('default: 22.23.2');
    expect(read('extensions/tmt-colab/rust/tmt-colab/tests/emulator/Dockerfile')).toContain(
      'node:22.23.2-'
    );
  });

  it.each([
    { build: 0, suite: 0, cleanup: 0, status: 0 },
    { build: 23, suite: 0, cleanup: 0, status: 23 },
    { build: 0, suite: 7, cleanup: 0, status: 7 },
    { build: 0, suite: 0, cleanup: 9, status: 1 },
  ])('propagates Colab Docker build/suite/cleanup status $status', (fixture) => {
    const ci = readFileSync(new URL('../../../.github/workflows/ci.yml', import.meta.url), 'utf8');
    const step = ci
      .split('      - name: Run the Colab Firestore Rules emulator suite\n')[1]
      .split('      - name: Run retained developer tooling tests')[0];
    const script = step.split('        run: |\n')[1].replace(/^ {10}/gm, '');
    const root = mkdtempSync(path.join(tmpdir(), 'tmt-colab-rules-ci-'));
    try {
      writeExecutable(
        path.join(root, 'docker'),
        `#!/bin/sh
printf '%s\\n' "$*" >> "$CALLS"
case "$1" in
  build) exit "$BUILD_STATUS" ;;
  run) exit "$SUITE_STATUS" ;;
  image) exit "$CLEANUP_STATUS" ;;
  *) exit 99 ;;
esac
`
      );
      runPackedCommand('bash', ['--noprofile', '--norc', '-eo', 'pipefail', '-c', script], {
        cwd: root,
        env: {
          ...process.env,
          PATH: `${root}:${process.env.PATH}`,
          GITHUB_RUN_ID: '123',
          GITHUB_RUN_ATTEMPT: '1',
          GITHUB_WORKSPACE: root,
          CALLS: path.join(root, 'calls'),
          BUILD_STATUS: String(fixture.build),
          SUITE_STATUS: String(fixture.suite),
          CLEANUP_STATUS: String(fixture.cleanup),
        },
        expectedStatus: fixture.status,
      });
      const calls = readFileSync(path.join(root, 'calls'), 'utf8').trim().split('\n');
      expect(calls[0]).toBe(
        'build --tag tmt-colab-rules:123-1 extensions/tmt-colab/rust/tmt-colab/tests/emulator'
      );
      if (fixture.build !== 0) {
        expect(calls).toHaveLength(1);
      } else {
        expect(calls).toHaveLength(3);
        expect(calls[1]).toContain(
          `run --rm --init --network none --volume ${root}/extensions/tmt-colab:/c:ro tmt-colab-rules:123-1 firebase emulators:exec`
        );
        expect(calls[2]).toBe('image rm tmt-colab-rules:123-1');
      }
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it('requires mapped Colab ownership and does not expand empty advisory scope', () => {
    expect(selectColabHarness([])).toBe(false);
    const map = parseComponentMap(
      JSON.stringify({
        components: { cli: { owns: ['.'] } },
        rules: [],
      })
    );
    expect(
      selectColabHarness(['extensions/tmt-colab/typescript/colab-client/src/index.ts'], map)
    ).toBe(false);
  });

  it('selects cumulative merge-group endpoints and fails closed when the diff is unreadable', () => {
    const root = mkdtempSync(path.join(tmpdir(), 'tmt-ci-queue-'));
    const git = (args: string[]) =>
      runPackedCommand('git', args, {
        cwd: root,
        env: { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null' },
      });
    const commit = (name: string, contents: string) => {
      mkdirSync(path.dirname(path.join(root, name)), { recursive: true });
      writeFileSync(path.join(root, name), contents);
      git(['add', '.']);
      git([
        '-c',
        'user.name=TMT Test',
        '-c',
        'user.email=test@example.invalid',
        '-c',
        'commit.gpgsign=false',
        'commit',
        '--quiet',
        '-m',
        'Test fixture',
      ]);
      return git(['rev-parse', 'HEAD']).trim();
    };
    const select = (args: string[], cwd = root) => {
      let output = '';
      let evidence = '';
      runCiScope(args, {
        cwd,
        stdout: {
          write: (text) => {
            output += text;
          },
        },
        stderr: {
          write: (text) => {
            evidence += text;
          },
        },
      });
      return {
        evidence,
        outputs: Object.fromEntries(
          output
            .trim()
            .split('\n')
            .map((line) => line.split('='))
        ),
      };
    };
    try {
      git(['init', '--quiet']);
      const base = commit('README.md', 'base');
      git(['update-ref', 'refs/remotes/origin/main', base]);
      commit('.agents/first.md', 'first PR');
      const docs = commit('.agents/second.md', 'second PR');
      const docsRun = select(['merge-group', docs]);
      expect(docsRun.outputs).toMatchObject({
        native: 'false',
        native_scope: 'none',
        e2e_shard_1: '',
        e2e_shard_2: '',
      });
      expect(docsRun.evidence).toContain(`${base.slice(0, 12)}..${docs.slice(0, 12)}`);
      expect(docsRun.evidence).toContain('.agents/first.md');
      expect(docsRun.evidence).toContain('.agents/second.md');
      expect(docsRun.outputs).toEqual(select([base, docs]).outputs);
      const ops = commit('extensions/tmt-ops/rust/tmt-ops/src/main.rs', '// ops');
      const opsRun = select(['merge-group', ops]);
      expect(opsRun.outputs).toMatchObject({
        native: 'true',
        native_scope: 'ops',
        e2e_shard_2: '',
      });
      expect(opsRun.outputs.e2e_shard_1).toBe('ops.e2e.test.ts ops-reminder.e2e.test.ts');
      expect(opsRun.outputs).toEqual(select([base, ops]).outputs);
      const shared = commit('unknown-input', 'shared');
      const full = select(['seed']).outputs;
      expect(select(['full']).outputs).toMatchObject({});
      const office = commit('extensions/tmt-office/rust/tmt-office/src/main.rs', '// Office');
      expect(select([shared, office]).outputs).toMatchObject({});
      expect(select(['merge-group', office]).outputs).toMatchObject({
        colab_harness: 'false',
        colab_app: 'false',
        colab_hosted: 'false',
        colab_hosted_emulator: 'false',
      });
      expect(select(['merge-group', shared]).outputs).toEqual(full);
      git(['update-ref', 'refs/remotes/origin/main', docs]);
      const empty = select(['merge-group', docs]);
      git(['update-ref', 'refs/remotes/origin/main', base]);
      expect(empty.outputs).toEqual({ ...full, native_notices: 'true' });
      expect(empty.evidence).toContain('diff is empty; using full verification');
      for (const args of [
        ['merge-group', '0'.repeat(40)],
        ['merge-group', '--help'],
        ['merge-group', base, docs],
        ['merge-group'],
      ]) {
        const fallback = select(args);
        expect(fallback.outputs).toEqual({ ...full, native_notices: 'true' });
        expect(fallback.evidence).toContain('diff unreadable; using full verification');
      }
      const colabRules = commit('extensions/tmt-colab/firestore/admission.rules', '// rules');
      const colabQueue = select(['merge-group', colabRules]);
      expect(colabQueue.outputs).toMatchObject({ native_scope: 'full', colab_firestore: 'true' });
      expect(colabQueue.outputs).toEqual(select([base, colabRules]).outputs);
      expect(select(['full']).outputs.colab_firestore).toBe('false');
      expect(select(['seed']).outputs.colab_firestore).toBe('false');
      const chromeHead = commit('extensions/tmt-colab/rust/tmt-colab/src/chrome.rs', '// chrome');
      const chromeQueue = select(['merge-group', chromeHead]);
      expect(chromeQueue.outputs.colab_chrome).toBe('true');
      expect(chromeQueue.outputs).toEqual(select([base, chromeHead]).outputs);
      expect(chromeQueue.evidence).toContain('Colab cross-screen chrome spec selection');
      expect(select(['full']).outputs.colab_chrome).toBe('false');
      expect(select(['seed']).outputs.colab_chrome).toBe('false');
      git(['checkout', '--quiet', '--detach', base]);
      commit('extensions/tmt-colab/typescript/app/src/style.css', 'browser');
      const browserTip = commit(
        'extensions/tmt-colab/typescript/app/e2e/reader-editor.spec.ts',
        'browser spec'
      );
      expect(select(['merge-group', browserTip]).outputs).toMatchObject({
        native_scope: 'none',
        colab_app: 'false',
      });
      expect(select([base, browserTip]).outputs).toMatchObject({
        native_scope: 'none',
        colab_app: 'true',
      });
      const native = commit('rust/native.rs', '// earlier queued native input');
      const browserOnlyTip = commit(
        'extensions/tmt-colab/typescript/app/test/reader.test.ts',
        'browser-only tip'
      );
      expect(select([native, browserOnlyTip]).outputs.native_scope).toBe('none');
      expect(select(['merge-group', browserOnlyTip]).outputs.native_scope).toBe('full');
      expect(select(['merge-group', browserOnlyTip]).evidence).toContain('rust/native.rs');
      git(['checkout', '--quiet', '--detach', base]);
      const contract = commit('extensions/tmt-colab/contracts/colab-v1.md', 'contract prose');
      expect(select(['merge-group', contract]).outputs.native_scope).toBe('none');
      const vector = commit('extensions/tmt-colab/contracts/vectors/fixture.json', '{}');
      const contractTip = commit('extensions/tmt-colab/contracts/colab-v1.md', 'new prose');
      expect(select([vector, contractTip]).outputs.native_scope).toBe('none');
      expect(select(['merge-group', contractTip]).outputs.native_scope).toBe('full');
      expect(select(['merge-group', contractTip]).evidence).toContain(
        'extensions/tmt-colab/contracts/vectors/fixture.json'
      );
      // The earlier queued workspace change selects native, even when HEADGREEN's last tip is site-only.
      git(['checkout', '--quiet', '--detach', base]);
      mkdirSync(path.join(root, 'rust'), { recursive: true });
      writeFileSync(
        path.join(root, 'rust/Cargo.toml'),
        '[workspace.package]\nversion = "5.0.0-alpha.35"\n'
      );
      writeFileSync(
        path.join(root, 'rust/Cargo.lock'),
        '[[package]]\nname = "tmt-cli"\nversion = "5.0.0-alpha.35"\n'
      );
      const workspace = commit(
        'rust/Cargo.toml',
        '[workspace.package]\nversion = "5.0.0-alpha.35"\n'
      );
      const siteTip = commit('site/src/chapters/start.mdx', 'site-only tip');
      // The old event-base range contains no native files: this is the causal negative control.
      expect(select([workspace, siteTip]).outputs.native_scope).toBe('none');
      expect(select([workspace, siteTip]).outputs.native_notices).toBe('false');
      const pending = select(['merge-group', siteTip]);
      expect(pending.outputs).toEqual({ ...full, native_notices: 'true' });
      expect(pending.evidence).toContain(`${base.slice(0, 12)}..${siteTip.slice(0, 12)}`);
      for (const file of ['rust/Cargo.toml', 'rust/Cargo.lock']) {
        expect(pending.evidence).toContain(file);
      }
      git(['update-ref', '-d', 'refs/remotes/origin/main']);
      const absentTarget = select(['merge-group', siteTip]);
      expect(absentTarget.outputs).toEqual({ ...full, native_notices: 'true' });
      expect(absentTarget.evidence).toContain('diff unreadable; using full verification');
      git(['update-ref', 'refs/remotes/origin/main', base]);
      // Divergent target/head histories use their common ancestor, excluding target-only work.
      git(['checkout', '--quiet', '--detach', base]);
      const core = commit('rust/fixture.rs', '// core');
      git(['update-ref', 'refs/remotes/origin/main', core]);
      expect(select(['merge-group', docs]).outputs.native_scope).toBe('none');
      expect(select([core, docs]).outputs.native_scope).toBe('none');
      // A real shallow checkout lacks the base object: it must not become an empty/none diff.
      const shallow = path.join(root, 'shallow');
      git([
        '-c',
        'advice.detachedHead=false',
        'clone',
        '--quiet',
        '--depth=1',
        `file://${root}`,
        shallow,
      ]);
      const fallback = select(['merge-group', core], shallow);
      expect(fallback.outputs).toEqual({ ...full, native_notices: 'true' });
      expect(fallback.evidence).toContain('diff unreadable; using full verification');
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  }, 5000);

  it('propagates gate failure to the command exit instead of only returning a boolean', () => {
    const script = fileURLToPath(new URL('../../scripts/ci-scope.mjs', import.meta.url));
    const options = { cwd: tmpdir(), env: process.env };
    expect(runPackedCommand(process.execPath, [script, 'gate', 'true', 'success'], options)).toBe(
      ''
    );
    expect(() =>
      runPackedCommand(process.execPath, [script, 'gate', 'true', 'skipped'], options)
    ).toThrow('Selected CI work did not complete successfully');
  });
});

describe('required CI gate', () => {
  it('accepts only successful selected work or explicitly unselected skipped work', () => {
    expect(ciGatePasses('true', ['success', 'success'])).toBe(true);
    expect(ciGatePasses('false', ['skipped', 'skipped'])).toBe(true);
  });

  it.each(['failure', 'cancelled', 'skipped', '', 'unknown'])(
    'rejects selected result %s',
    (result) => {
      expect(ciGatePasses('true', ['success', result])).toBe(false);
    }
  );

  it('rejects missing selection/results and contradictions rather than claiming a pass', () => {
    expect(ciGatePasses('', ['skipped'])).toBe(false);
    expect(ciGatePasses('true', [])).toBe(false);
    expect(ciGatePasses('false', ['success'])).toBe(false);
    expect(ciGatePasses('false', ['failure'])).toBe(false);
  });

  it.each([
    {
      selection: 'Office only',
      office: ['true', ['success', 'success']] as const,
      native: ['false', ['skipped', 'skipped', 'skipped', 'skipped', 'skipped']] as const,
    },
    {
      selection: 'native only',
      office: ['false', ['skipped']] as const,
      native: ['true', ['success', 'success', 'success', 'success', 'success', 'success']] as const,
    },
    {
      selection: 'Office and native',
      office: ['true', ['success', 'success']] as const,
      native: ['true', ['success', 'success', 'success', 'success', 'success', 'success']] as const,
    },
    {
      selection: 'neither',
      office: ['false', ['skipped']] as const,
      native: ['false', ['skipped', 'skipped', 'skipped', 'skipped', 'skipped']] as const,
    },
  ])('accepts the complete $selection partition result', ({ office, native }) => {
    expect(ciGatePasses(native[0], [...native[1]])).toBe(true);
  });

  /** The results of the eight native jobs, all `success` unless overridden. */
  const native = (overrides: Record<string, string> = {}) => ({
    nativeRust: 'success',
    unitTests: 'success',
    e2eShard1: 'success',
    e2eShard2: 'success',
    runtimeBuild: 'success',
    packedInstall: 'success',
    macosRuntimeBuild: 'success',
    macosPackedInstall: 'success',
    ...overrides,
  });
  const allSkipped = {
    nativeRust: 'skipped',
    unitTests: 'skipped',
    e2eShard1: 'skipped',
    e2eShard2: 'skipped',
    runtimeBuild: 'skipped',
    packedInstall: 'skipped',
    macosRuntimeBuild: 'skipped',
    macosPackedInstall: 'skipped',
  };
  const opsRun = native({
    unitTests: 'skipped',
    e2eShard2: 'skipped',
    runtimeBuild: 'skipped',
    packedInstall: 'skipped',
    macosRuntimeBuild: 'skipped',
    macosPackedInstall: 'skipped',
  });

  it.each([
    ['full', native()],
    ['ops', opsRun],
    ['none', allSkipped],
  ])('accepts exactly the %s native results', (scope, expected) => {
    expect(nativeGatePasses(scope, expected, 'true')).toBe(true);
  });

  it.each([
    ['full', native({ unitTests: 'skipped' }), 'a skipped job that must run'],
    ['full', native({ packedInstall: 'failure' }), 'a failed job'],
    ['full', native({ packedInstall: 'cancelled' }), 'a cancelled job'],
    ['full', native({ e2eShard2: 'skipped' }), 'a skipped second E2E shard'],
    ['full', native({ e2eShard1: 'failure' }), 'a failed first E2E shard'],
    ['ops', { ...opsRun, unitTests: 'success' }, 'unit tests that must be skipped'],
    ['ops', { ...opsRun, e2eShard1: 'skipped' }, 'a skipped E2E shard'],
    ['ops', { ...opsRun, e2eShard2: 'success' }, 'a second E2E shard that must be skipped'],
    ['ops', { ...opsRun, nativeRust: 'skipped' }, 'a skipped native contract'],
    ['ops', { ...opsRun, runtimeBuild: 'success' }, 'a runtime build that must be skipped'],
    ['ops', { ...opsRun, e2eShard1: 'failure' }, 'a failed E2E shard'],
    ['none', { ...allSkipped, nativeRust: 'success' }, 'work when nothing was selected'],
    ['unknown', native(), 'an unknown scope'],
    ['', allSkipped, 'an empty scope'],
    ['office', opsRun, 'a component without scoped checks'],
    ['ops', { ...opsRun, packedInstall: '' }, 'an empty result'],
  ])('rejects %s with %o (%s)', (scope, actual) => {
    expect(nativeGatePasses(scope, actual, 'true')).toBe(false);
  });

  it('allows exactly the macOS omissions on merge groups and rejects missing evidence', () => {
    const queue = native({ macosRuntimeBuild: 'skipped', macosPackedInstall: 'skipped' });
    expect(nativeGatePasses('full', queue, 'false')).toBe(true);
    expect(nativeGatePasses('full', queue, 'true')).toBe(false);
    expect(nativeGatePasses('full', native(), 'false')).toBe(false);
    for (const [job, expected] of Object.entries(queue)) {
      for (const result of ['success', 'skipped', 'failure', 'cancelled', '', undefined]) {
        if (result === expected) continue;
        expect(nativeGatePasses('full', { ...queue, [job]: result }, 'false')).toBe(false);
      }
    }
    for (const macos of ['', 'unknown', undefined]) {
      expect(nativeGatePasses('full', queue, macos as string)).toBe(false);
      expect(nativeGatePasses('none', allSkipped, macos as string)).toBe(false);
      expect(nativeGatePasses('ops', opsRun, macos as string)).toBe(false);
    }
    expect(nativeGatePasses('ops', opsRun, 'false')).toBe(true);
    expect(nativeGatePasses('none', allSkipped, 'false')).toBe(true);
  });

  it('rejects missing native results rather than claiming a pass', () => {
    expect(nativeGatePasses('full', {} as never, 'true')).toBe(false);
    expect(nativeGatePasses('none', undefined as never, 'true')).toBe(false);
    const { e2eShard2: _missing, ...withoutShard } = native();
    expect(nativeGatePasses('full', withoutShard as never, 'true')).toBe(false);
  });
  describe('Native Rust contracts gate', () => {
    const expectedFor = (scope: string) =>
      Array<string>(4).fill(scope === 'none' ? 'skipped' : 'success');
    it.each(['full', 'ops', 'none'])('requires exactly the selected workers for %s', (scope) => {
      const expected = expectedFor(scope);
      expect(rustGatePasses(scope, expected)).toBe(true);
      for (let index = 0; index < expected.length; index++)
        for (const value of ['success', 'skipped', 'failure', 'cancelled', '', undefined]) {
          if (value === expected[index]) continue;
          const changed = [...expected];
          changed[index] = value as string;
          expect(rustGatePasses(scope, changed)).toBe(false);
        }
      for (const results of [[], expected.slice(1), [...expected, 'success'], undefined])
        expect(rustGatePasses(scope, results as string[])).toBe(false);
    });
    it.each(['', 'unknown', 'office'])('rejects unsupported scope %s', (scope) =>
      expect(rustGatePasses(scope, expectedFor('full'))).toBe(false)
    );
    describe('job condition', () => {
      // Evaluates the job's `if:` for the changes outputs, supporting only the `always()` and
      // `needs.changes.outputs.<name> != '<literal>'` terms that the condition is made of.
      const jobRuns = (outputs: Record<string, string>) => {
        const workflow = readFileSync(
          fileURLToPath(new URL('../../../.github/workflows/ci.yml', import.meta.url)),
          'utf8'
        );
        const start = workflow.indexOf('\n  native-rust:\n');
        const condition = /^ {4}if: \$\{\{ (.*) \}\}$/m.exec(workflow.slice(start))?.[1] ?? '';
        const terms = condition.split(' && ');
        expect(terms.length).toBeGreaterThan(1);
        return terms.every((term) => {
          if (term === 'always()') return true;
          const match = /^needs\.changes\.outputs\.(\w+) != '(\w+)'$/.exec(term);
          if (!match) throw new Error(`Unsupported condition term: ${term}`);
          return (outputs[match[1]] ?? '') !== match[2];
        });
      };

      it('skips a superseded main run, whose workers were skipped with verify=false', () => {
        expect(jobRuns({ native_scope: 'full', verify: 'false' })).toBe(false);
      });
      it('skips the gate for scope none and keeps running it for a verified selection', () => {
        expect(jobRuns({ native_scope: 'none', verify: 'true' })).toBe(false);
        for (const scope of ['full', 'ops'])
          expect(jobRuns({ native_scope: scope, verify: 'true' })).toBe(true);
      });
      it('still runs, and fails, when the selection is missing', () => {
        // The changes job failed: every output is empty, all workers were skipped.
        expect(jobRuns({})).toBe(true);
        expect(rustGatePasses('', Array<string>(4).fill('skipped'))).toBe(false);
      });
    });
    it('enforces missing, extra and failed workers through the actual command', () => {
      const script = fileURLToPath(new URL('../../scripts/ci-scope.mjs', import.meta.url));
      const options = { cwd: process.cwd(), env: process.env };
      for (const scope of ['full', 'ops', 'none']) {
        const expected = expectedFor(scope);
        expect(
          runPackedCommand(process.execPath, [script, 'gate-rust', scope, ...expected], options)
        ).toBe('');
        const wrong = expected.map((_, index) =>
          expected.map((value, i) => (i === index ? 'failure' : value))
        );
        for (const values of [expected.slice(1), [...expected, 'success'], ...wrong])
          expect(() =>
            runPackedCommand(process.execPath, [script, 'gate-rust', scope, ...values], options)
          ).toThrow('Selected native Rust workers');
      }
    });
  });

  describe('Docker E2E gate', () => {
    it.each([
      ['full', 'success', 'success'],
      ['ops', 'success', 'skipped'],
      ['none', 'skipped', 'skipped'],
    ])(
      'passes for %s only with the results the selector implies (%s, %s)',
      (scope, first, second) => {
        expect(e2eGatePasses(scope, { e2eShard1: first, e2eShard2: second })).toBe(true);
      }
    );

    it.each([
      ['full', 'skipped', 'success', 'a selected shard that was skipped'],
      ['full', 'success', 'skipped', 'a selected shard that was skipped'],
      ['full', 'skipped', 'skipped', 'both shards skipped although native work was selected'],
      ['full', 'cancelled', 'success', 'a cancelled shard'],
      ['full', 'success', 'cancelled', 'a cancelled shard'],
      ['full', 'failure', 'success', 'a failed shard'],
      ['full', 'success', '', 'a missing shard result'],
      ['full', '', '', 'both results missing'],
      ['ops', 'skipped', 'skipped', 'the Ops shard skipped'],
      ['ops', 'cancelled', 'skipped', 'the Ops shard cancelled'],
      ['ops', 'success', 'success', 'a second shard that must be skipped'],
      ['ops', 'success', 'failure', 'a second shard that ran and failed'],
      ['none', 'success', 'skipped', 'a shard that ran although nothing was selected'],
      ['none', 'skipped', 'success', 'a shard that ran although nothing was selected'],
      ['none', 'failure', 'failure', 'failed shards although nothing was selected'],
      ['', 'skipped', 'skipped', 'an unavailable selector scope'],
      ['unknown', 'success', 'success', 'an unknown scope'],
    ])('rejects %s with (%s, %s): %s', (scope, first, second) => {
      expect(e2eGatePasses(scope, { e2eShard1: first, e2eShard2: second })).toBe(false);
    });

    it('rejects results that are not there at all', () => {
      expect(e2eGatePasses('full', {} as never)).toBe(false);
      expect(e2eGatePasses('full', { e2eShard1: 'success' } as never)).toBe(false);
      expect(e2eGatePasses('none', undefined as never)).toBe(false);
    });

    it('fails the command with the same rules, and needs exactly two shard results', () => {
      const io = () => ({
        cwd: fileURLToPath(new URL('../../../', import.meta.url)),
        stdout: { write: () => {} },
        stderr: { write: () => {} },
      });
      runCiScope(['gate-e2e', 'full', 'success', 'success'], io());
      runCiScope(['gate-e2e', 'ops', 'success', 'skipped'], io());
      runCiScope(['gate-e2e', 'none', 'skipped', 'skipped'], io());
      for (const args of [
        ['gate-e2e', 'full', 'success', 'skipped'],
        ['gate-e2e', 'full', 'success'],
        ['gate-e2e', 'full', 'success', 'success', 'success'],
        ['gate-e2e', 'none', 'success', 'skipped'],
        ['gate-e2e', 'full', 'success', 'cancelled'],
        ['gate-e2e', '', 'skipped', 'skipped'],
      ]) {
        expect(() => runCiScope(args, io()), args.join(' ')).toThrow('Docker E2E shards');
      }
      const queueResults = [
        'success',
        'success',
        'success',
        'success',
        'success',
        'success',
        'skipped',
        'skipped',
      ];
      runCiScope(['gate-native', 'false', 'full', ...queueResults], io());
      for (const macos of ['true', '', 'unknown']) {
        expect(() => runCiScope(['gate-native', macos, 'full', ...queueResults], io())).toThrow(
          'native CI work'
        );
      }
      expect(() =>
        runCiScope(['gate-native', 'false', 'full', ...queueResults.slice(1)], io())
      ).toThrow('native CI work');
      runCiScope(
        [
          'gate-native',
          'true',
          'ops',
          'success',
          'skipped',
          'success',
          'skipped',
          'skipped',
          'skipped',
          'skipped',
          'skipped',
        ],
        io()
      );
      expect(() =>
        runCiScope(
          [
            'gate-native',
            'true',
            'full',
            'success',
            'success',
            'success',
            'skipped',
            'success',
            'success',
            'success',
            'success',
          ],
          io()
        )
      ).toThrow('native CI work');
    });
  });

  it('fails both stable aggregates when selector output is unavailable', () => {
    expect(ciGatePasses('', ['skipped'])).toBe(false);
    expect(ciGatePasses('', ['skipped', 'skipped', 'skipped', 'skipped', 'skipped'])).toBe(false);
  });

  it('keeps all PR runtime rows but skips only the separate macOS jobs on merge groups', () => {
    const workflow = readFileSync(
      fileURLToPath(new URL('../../../.github/workflows/ci.yml', import.meta.url)),
      'utf8'
    );
    const job = (name: string) => {
      const start = workflow.indexOf(`\n  ${name}:\n`);
      expect(start).toBeGreaterThan(0);
      const next = workflow.slice(start + 1).search(/\n {2}[a-z0-9-]+:\n/);
      return workflow.slice(start, next < 0 ? undefined : start + 1 + next);
    };
    const build = job('native-runtime-build');
    const macBuild = job('native-runtime-build-macos');
    const packed = job('packed-native-install');
    const macPacked = job('packed-native-install-macos');
    expect(build.match(/- target: (.+)/g)).toEqual([
      '- target: x86_64-unknown-linux-musl',
      '- target: aarch64-unknown-linux-musl',
    ]);
    expect(macBuild.match(/- target: (.+)/g)).toEqual([
      '- target: x86_64-apple-darwin',
      '- target: aarch64-apple-darwin',
    ]);
    expect(packed.match(/- label: (.+)/g)).toEqual([
      '- label: Linux glibc x64',
      '- label: Linux glibc arm64',
      '- label: Linux musl x64',
      '- label: Linux musl arm64',
    ]);
    expect(macPacked.match(/- label: (.+)/g)).toEqual([
      '- label: macOS x64',
      '- label: macOS arm64',
    ]);
    expect(macBuild).toContain(
      "if: needs.changes.outputs.macos == 'true' && needs.changes.outputs.native_scope == 'full'"
    );
    expect(macPacked).toContain(
      "if: needs.changes.outputs.macos == 'true' && needs.changes.outputs.verify == 'true' && needs.changes.outputs.native_scope == 'full'"
    );
    expect(macPacked).toContain('needs: [changes, native-runtime-build-macos]');
    expect(packed).toContain('needs: [changes, native-runtime-build]');
    expect(build).toContain('steps: &native-runtime-build-steps');
    expect(macBuild).toContain('steps: *native-runtime-build-steps');
    expect(packed).toContain('steps: &packed-native-install-steps');
    expect(macPacked).toContain('steps: *packed-native-install-steps');
    for (const section of [build, macBuild]) {
      expect(section).toContain('name: Build native runtime (${{ matrix.target }})');
    }
    for (const section of [packed, macPacked]) {
      expect(section).toContain('name: Packed install (${{ matrix.label }})');
    }
    const gate = job('native-install-gate');
    expect(gate).toContain('MACOS_SELECTED: ${{ needs.changes.outputs.macos }}');
    for (const name of ['native-runtime-build-macos', 'packed-native-install-macos']) {
      expect(gate).toContain(`        ${name},`);
    }
  });

  it('builds and transfers the driver-owned companion without replacing debug Rust verification', () => {
    const workflow = readFileSync(
      fileURLToPath(new URL('../../../.github/workflows/ci.yml', import.meta.url)),
      'utf8'
    );
    const start = workflow.indexOf('\n  native-clippy:\n');
    const native = workflow.slice(start, workflow.indexOf('\n  unit-tests:\n', start));
    expect(
      native.match(/cargo build --locked --release -p tmt-cli -p tmt-driver-herdr --bins/g)
    ).toHaveLength(2);
    const runtime = workflow
      .split('\n  native-runtime-build:\n')[1]
      .split('\n  native-runtime-build-macos:\n')[0];
    expect(runtime).toContain(
      "cargo build --locked --release --target '${{ matrix.target }}' -p tmt-cli -p tmt-driver-herdr --bins"
    );
    expect(runtime).toContain('rust/target/${{ matrix.target }}/release/tmt-driver-herdr');
    expect(runtime).toContain(
      "cargo build --locked --release --target '${{ matrix.target }}' -p tmt-test-support --example colab-runtime-fixture"
    );
    expect(runtime).toContain(
      'rust/target/${{ matrix.target }}/release/examples/colab-runtime-fixture'
    );
    const tooling = workflow.split('\n  unit-tests:\n')[1].split('\n  docker-e2e-shard-1:\n')[0];
    expect(tooling).toContain('name: runtime-x86_64-unknown-linux-musl');
    expect(tooling).toContain('path: rust/target/debug');
    expect(tooling).toContain(
      'chmod +x ../rust/target/debug/tmt ../rust/target/debug/tmt-driver-herdr'
    );
    expect(tooling).toContain('../rust/target/debug/examples/colab-runtime-fixture');
    expect(native).toContain('rust/target/release/tmt');
    const workspace = readFileSync(
      new URL('../../../.github/workflows/workspace-partitions.yml', import.meta.url),
      'utf8'
    );
    expect(workspace).toContain('cargo test --locked --workspace --doc');
    expect(workspace).toContain('workspace-nextest.mjs inventory');
    expect(native).toContain('cargo clippy --locked --workspace --all-targets -- -D warnings');
    expect(native).toContain('cargo +"$MSRV" check --locked --workspace --all-targets');
    expect(native).not.toMatch(/cargo \+\d/);
    expect(native).toContain('rust/target/debug/examples/storage-probe');
    expect(native).toContain('cargo build --locked --example storage-probe');
    expect(native).toContain('cargo fmt --all --check');
    expect(native).toContain('pnpm test:native --reporter=verbose');
  });
  it('keeps browser diagnostics outside required aggregates while required results fail closed', () => {
    const workflow = readFileSync(
      fileURLToPath(new URL('../../../.github/workflows/ci.yml', import.meta.url)),
      'utf8'
    );
    const codeQuality = workflow.slice(
      workflow.indexOf('\n  code-quality:\n'),
      workflow.indexOf('\n  native-rust:\n')
    );
    const native = workflow.slice(workflow.indexOf('\n  native-install-gate:\n'));

    expect(codeQuality).toContain('needs: changes');
    expect(native).toContain('native-rust');
    expect(native).toContain('unit-tests');
    expect(native).toContain('docker-e2e');
    expect(native).toContain('native-runtime-build');
    expect(native).toContain('packed-native-install');
    expect(native).not.toContain('native-office-browser');
    expect(native).not.toContain('BROWSER_RESULT');

    expect(ciGatePasses('true', ['success'])).toBe(true);
    expect(ciGatePasses('true', ['failure'])).toBe(false);
    expect(ciGatePasses('true', ['failure', 'success', 'success', 'success', 'success'])).toBe(
      false
    );
  });

  it('keeps complete disjoint fallback shards and wires merge-group selection with read-only caches', () => {
    const capture = () => {
      let value = '';
      return {
        write: (text: string) => {
          value += text;
        },
        text: () => value,
      };
    };
    const stdout = capture();
    const stderr = capture();
    runCiScope(['full'], { cwd: '/no-diff-required', stdout, stderr });
    const outputs: Record<string, string> = Object.fromEntries(
      stdout
        .text()
        .trim()
        .split('\n')
        .map((line) => line.split('='))
    );
    expect(outputs).toMatchObject({ native: 'true', native_scope: 'full' });
    const first = outputs.e2e_shard_1.split(' ');
    const second = outputs.e2e_shard_2.split(' ');
    expect(first.length).toBeGreaterThan(0);
    expect(second.length).toBeGreaterThan(0);
    expect(first.filter((file) => second.includes(file))).toEqual([]);
    expect([...first, ...second].sort()).toEqual(listE2eFiles());
    expect(stderr.text()).toContain('Weekly/manual retained-product verification.');
    const workflow = readFileSync(
      new URL('../../../.github/workflows/ci.yml', import.meta.url),
      'utf8'
    );
    expect(workflow).toContain('\n  merge_group:\n');
    expect(workflow).toContain(
      "VERIFY: ${{ github.event_name == 'pull_request' || github.event_name == 'merge_group' }}"
    );
    expect(workflow).toContain("if: steps.event.outputs.verify != 'true'");
    expect(workflow).toContain(
      'verify: ${{ steps.main.outputs.verify || steps.event.outputs.verify }}'
    );
    expect(workflow).toContain(
      'macos: ${{ steps.main.outputs.macos || steps.event.outputs.macos }}'
    );
    expect(workflow).toContain("MACOS: ${{ github.event_name != 'merge_group' }}");
    expect(workflow).toContain('fetch-depth: 0');
    expect(workflow).not.toContain('github.event.merge_group.base_sha');
    expect(workflow).toContain('HEAD_SHA: ${{ github.event.merge_group.head_sha }}');
    expect(workflow).toContain(
      'run: node typescript/scripts/ci-scope.mjs merge-group "$HEAD_SHA" >> "$GITHUB_OUTPUT"'
    );
    const cachePolicies = workflow.match(/^\s+save-if:.*$/gm) ?? [];
    const writers = cachePolicies.filter((line) => line.trim() !== 'save-if: false');
    expect(cachePolicies).toHaveLength(6);
    expect(writers).toHaveLength(3);
    for (const writer of writers) {
      expect(writer.trim()).toBe(
        "save-if: ${{ github.ref == 'refs/heads/main' && (needs.changes.outputs.verify == 'false' || needs.changes.outputs.main_full_admitted == 'true') }}"
      );
    }
    for (const name of ['code-quality', 'docker-e2e', 'native-install-gate']) {
      const body = workflow.split(`\n  ${name}:\n`)[1].split(/\n {2}[a-z0-9-]+:\n/)[0];
      expect(body).toContain("always() && needs.changes.outputs.verify != 'false'");
    }
  });
  it('normalizes every CI cache consumer and Colab through one install owner before restore', () => {
    const workflow = readFileSync(
      new URL('../../../.github/workflows/ci.yml', import.meta.url),
      'utf8'
    );
    const sites = [
      'native-notices',
      'colab-chrome',
      'native-clippy',
      'native-process-tests',
      'native-msrv',
      'native-runtime-build',
    ];
    const workspace = readFileSync(
      new URL('../../../.github/workflows/workspace-partitions.yml', import.meta.url),
      'utf8'
    );
    expect((workflow + workspace).match(/sh scripts\/install-ci-rust\.sh /g)).toHaveLength(
      sites.length + 2
    );
    expect(workspace.indexOf('sh scripts/install-ci-rust.sh ')).toBeLessThan(
      workspace.indexOf('uses: Swatinem/rust-cache@')
    );
    expect(workspace).toContain('install-ci-rust.sh 1.97.0 --profile minimal');
    expect(workflow).not.toContain('add-rust-environment-hash-key: false');
    for (const name of sites) {
      const body = workflow.split(`\n  ${name}:\n`)[1].split(/\n {2}[a-z0-9-]+:\n/)[0];
      const install = body.indexOf('sh scripts/install-ci-rust.sh ');
      expect(install, name).toBeGreaterThan(0);
      expect(install, name).toBeLessThan(body.indexOf('uses: Swatinem/rust-cache@'));
      expect(body).toContain(
        name === 'native-msrv'
          ? 'install-ci-rust.sh "$MSRV" --profile minimal'
          : 'install-ci-rust.sh 1.97.0 --profile minimal'
      );
    }
    expect(workflow).toContain('steps: &native-runtime-build-steps');
    expect(workflow).toContain('steps: *native-runtime-build-steps');
    expect(workflow).toContain(
      "install-ci-rust.sh 1.97.0 --profile minimal --target '${{ matrix.target }}'"
    );
    expect(
      (workflow + workspace).match(
        /install-ci-rust\.sh 1\.97\.0 --profile minimal --component rustfmt,clippy/g
      )
    ).toHaveLength(4);
    const colab = readFileSync(
      new URL('../../../.github/workflows/colab-browser.yml', import.meta.url),
      'utf8'
    );
    const install = colab.indexOf('sh scripts/install-ci-rust.sh 1.97.0 --profile minimal');
    expect(colab.match(/sh scripts\/install-ci-rust\.sh /g)).toHaveLength(4);
    expect(install).toBeGreaterThan(0);
    expect(install).toBeLessThan(colab.indexOf('uses: Swatinem/rust-cache@'));
    expect(colab).toContain('shared-key: native-rust');
    expect(colab).toContain('save-if: false');
  });

  it('gives every native step and job an explicit scope, and gates on exactly those results', () => {
    const workflow = readFileSync(
      fileURLToPath(new URL('../../../.github/workflows/ci.yml', import.meta.url)),
      'utf8'
    );
    const job = (name: string) => {
      const start = workflow.indexOf(`\n  ${name}:\n`);
      const next = workflow.slice(start + 1).search(/\n {2}[a-z0-9-]+:\n/);
      return workflow.slice(start, next < 0 ? undefined : start + 1 + next);
    };
    const steps = (body: string) =>
      body
        .split('\n      - ')
        .slice(1)
        .map((step) => ({
          name: /^name: (.*)$/m.exec(step)?.[1] ?? '',
          scope: /^ {8}if: (.*)$/m.exec(step)?.[1],
          body: step,
        }));

    const changes = job('changes');
    for (const output of ['scoped_native_tests', 'e2e_shard_1', 'e2e_shard_2']) {
      expect(changes).toContain(
        `${output}: \${{ steps.mainfull.outputs.${output} || steps.scope.outputs.${output} || steps.queue.outputs.${output} }}`
      );
    }
    // A seeding run (no pull request, no diff) builds the whole workspace, so it is the full scope.
    expect(changes).toContain(
      'native_scope: ${{ steps.mainfull.outputs.native_scope || steps.scope.outputs.native_scope || steps.queue.outputs.native_scope || steps.seed.outputs.native_scope }}'
    );
    expect(changes).toContain('node typescript/scripts/ci-scope.mjs "$SEED_SCOPE"');
    expect(changes).toContain(
      "(github.event_name == 'schedule' || github.event_name == 'workflow_dispatch') && 'full' || 'seed'"
    );
    // Jobs about the CLI runtime and tooling run only for the full scope; the runtime build also
    // runs for the seeding runs, whose scope is full, and the other two only for verification events.
    expect(job('native-runtime-build')).toContain(
      "if: needs.changes.outputs.native_scope == 'full'"
    );
    for (const name of ['unit-tests', 'packed-native-install']) {
      expect(job(name), name).toContain(
        "if: needs.changes.outputs.verify == 'true' && needs.changes.outputs.native_scope == 'full'"
      );
    }
    // The Rust job runs for every native scope, the E2E job for every native scope of a pull request.
    for (const name of ['native-clippy', 'native-workspace-tests', 'native-msrv']) {
      expect(job(name)).toContain("if: needs.changes.outputs.native == 'true'");
    }
    const rustGate = job('native-rust');
    expect(rustGate.replace(/\s+/g, ' ')).toContain(
      'needs: [changes, native-clippy, native-workspace-tests, native-process-tests, native-msrv]'
    );
    expect(rustGate).toContain(
      "if: ${{ always() && needs.changes.outputs.native_scope != 'none' && needs.changes.outputs.verify != 'false' }}"
    );
    for (const [variable, worker] of [
      ['CLIPPY_RESULT', 'native-clippy'],
      ['TESTS_RESULT', 'native-workspace-tests'],
      ['NATIVE_RESULT', 'native-process-tests'],
    ])
      expect(rustGate).toContain(`${variable}: \${{ needs.${worker}.result }}`);
    expect(rustGate).toContain('MSRV_RESULT: ${{ needs.native-msrv.result }}');
    const rustVariables = {
      clippy: 'CLIPPY_RESULT',
      tests: 'TESTS_RESULT',
      process: 'NATIVE_RESULT',
      msrv: 'MSRV_RESULT',
    };
    const rustArguments = RUST_WORKERS.map((worker) => `"$${rustVariables[worker]}"`).join(' ');
    expect(rustGate).toContain(`ci-scope.mjs gate-rust "$NATIVE_SCOPE" ${rustArguments}`);
    expect(job('native-msrv')).toContain('shared-key: native-rust-msrv');
    expect(job('native-msrv')).toContain(
      "save-if: ${{ github.ref == 'refs/heads/main' && (needs.changes.outputs.verify == 'false' || needs.changes.outputs.main_full_admitted == 'true') }}"
    );
    expect(job('native-msrv')).toContain('["workspace"]["package"]["rust-version"]');
    expect(job('native-msrv')).toContain('RUSTUP_TOOLCHAIN=%s');
    expect(job('native-msrv')).toContain('sh scripts/install-ci-rust.sh "$MSRV" --profile minimal');
    expect(job('native-msrv')).not.toMatch(/install-ci-rust\.sh \d/);
    expect(job('native-msrv')).toContain('cargo +"$MSRV" check --locked --workspace --all-targets');
    // The E2E suite is two shard jobs; only the first runs for a scoped component, and
    // only the first of a full run also runs the Rust adapter tests.
    expect(job('docker-e2e-shard-1')).toContain(
      "if: needs.changes.outputs.verify == 'true' && needs.changes.outputs.native == 'true'"
    );
    expect(job('docker-e2e-shard-1')).toContain(
      'TMT_E2E_FILES: ${{ needs.changes.outputs.e2e_shard_1 }}'
    );
    expect(job('docker-e2e-shard-1')).toContain(
      "TMT_E2E_ADAPTER_TESTS: ${{ needs.changes.outputs.native_scope == 'full' && '1' || '0' }}"
    );
    expect(job('docker-e2e-shard-2')).toContain(
      "if: needs.changes.outputs.verify == 'true' && needs.changes.outputs.native_scope == 'full'"
    );
    expect(job('docker-e2e-shard-2')).toContain(
      'TMT_E2E_FILES: ${{ needs.changes.outputs.e2e_shard_2 }}'
    );
    expect(job('docker-e2e-shard-2')).toContain("TMT_E2E_ADAPTER_TESTS: '0'");
    // Both shards run the same steps (a YAML alias), so they cannot drift apart.
    expect(job('docker-e2e-shard-1')).toContain('steps: &docker-e2e-steps');
    expect(job('docker-e2e-shard-2')).toContain('steps: *docker-e2e-steps');
    const dockerSteps = job('docker-e2e-shard-1').split('steps: &docker-e2e-steps')[1];
    expect(dockerSteps).toContain('uses: actions/setup-node@v4');
    expect(dockerSteps).toContain('node-version: 22.23.2');
    expect(dockerSteps).toContain('run: node scripts/run-e2e.mjs');
    expect(dockerSteps).not.toMatch(/setup-tooling|pnpm|corepack|Install dependencies/);
    for (const name of ['docker-e2e-shard-1', 'docker-e2e-shard-2']) {
      expect(job(name).match(/^ {4}runs-on: (.+)$/gm)).toEqual(['    runs-on: ubuntu-latest']);
    }
    const builder = steps(job('docker-e2e-shard-1')).find((step) =>
      step.body.includes('id: e2e-mirror-builder')
    )!;
    expect(builder.body).toContain('cleanup: false');
    expect(builder.body).toContain(
      'docker/setup-buildx-action@e468171a9de216ec08956ac3ada2f0791b6bd435'
    );
    expect(builder.body).toContain('version: v0.29.1');
    expect(builder.body).toContain(
      'image=mirror.gcr.io/moby/buildkit:buildx-stable-1@sha256:cec9f139f45e93c5c69c60f8b07cfad9f43f4ef6b6a6cd917527fea5ff2e3dea'
    );
    const workflows = new URL('../../../.github/workflows/', import.meta.url);
    for (const file of readdirSync(workflows).filter((name) => /\.ya?ml$/.test(name))) {
      const disabled = steps(readFileSync(new URL(file, workflows), 'utf8')).filter((step) =>
        /cleanup:\s*false/.test(step.body)
      );
      expect(
        disabled.map((step) => step.name),
        file
      ).toEqual(file === 'ci.yml' ? ['Set up Buildx only for an admitted archive'] : []);
    }
    expect(readFileSync(new URL('../../scripts/run-e2e.mjs', import.meta.url), 'utf8')).toContain(
      "await run('docker', ['image', 'rm', '--force', image]"
    );

    // `Docker E2E`, the required check, is a gate over exactly the two shards.
    const e2eGate = job('docker-e2e');
    expect(e2eGate).toContain('name: Docker E2E\n');
    expect(e2eGate).toContain('needs: [changes, docker-e2e-shard-1, docker-e2e-shard-2]');
    // Like the other gates, only for verification events: a seeding run has no gate to satisfy.
    expect(e2eGate).toContain("if: ${{ always() && needs.changes.outputs.verify != 'false' }}");
    expect(e2eGate).toContain('SHARD_1_RESULT: ${{ needs.docker-e2e-shard-1.result }}');
    expect(e2eGate).toContain('SHARD_2_RESULT: ${{ needs.docker-e2e-shard-2.result }}');
    expect(e2eGate).toContain(
      'ci-scope.mjs gate-e2e "$NATIVE_SCOPE" "$SHARD_1_RESULT" "$SHARD_2_RESULT"'
    );
    // Each required check name belongs to exactly one job, and no shard reuses one.
    for (const required of [
      'Code quality',
      'Unit tests',
      'Docker E2E',
      'Native Rust contracts',
      'Native package matrix',
    ]) {
      expect(workflow.match(new RegExp(`^ {4}name: ${required}$`, 'gm')), required).toHaveLength(1);
    }
    expect(workflow.match(/^ {4}name: Docker E2E shard \d\/2$/gm)).toHaveLength(2);
    expect(job('unit-tests')).toContain(
      'name: runtime-x86_64-unknown-linux-musl\n          path: rust/target/debug'
    );
    expect(job('native-process-tests')).toContain('needs: changes');
    expect(job('native-process-tests')).toContain('timeout-minutes: 20');
    expect(job('native-process-tests')).toContain(
      'uses: ./.github/actions/apt-install\n        with:\n          packages: zsh'
    );
    expect(job('native-process-tests')).toContain(
      "always() && needs.changes.outputs.native == 'true'"
    );
    expect(job('native-clippy')).toContain('Reject an unknown native scope');
    const scoped = (worker: string, scope: string) =>
      steps(job(worker))
        .filter((step) => step.scope === `needs.changes.outputs.native_scope == '${scope}'`)
        .map((step) => step.name);
    expect(scoped('native-clippy', 'full')).toEqual(['Lint workspace']);
    expect(scoped('native-clippy', 'ops')).toEqual(['Lint Ops']);
    expect(job('native-workspace-tests')).toContain(
      'uses: ./.github/workflows/workspace-partitions.yml'
    );
    expect(job('native-workspace-tests')).toContain(
      'scope: ${{ needs.changes.outputs.native_scope }}'
    );
    expect(scoped('native-process-tests', 'full')).toEqual([
      'Build independent native process fixtures',
      'Verify native process and shared parser contracts',
    ]);
    expect(scoped('native-process-tests', 'ops')).toEqual([
      'Build Ops process fixtures',
      'Verify Ops native contracts',
    ]);
    // Both scopes run extension-install, whose archives use these debug executables.
    for (const scope of ['full', 'ops']) {
      const fixtures = steps(job('native-process-tests')).find(
        (step) =>
          step.scope === `needs.changes.outputs.native_scope == '${scope}'` &&
          step.name.startsWith('Build ')
      );
      const debugBuilds = [...(fixtures?.body.matchAll(/cargo build ([^\n]+)/g) ?? [])]
        .map((match) => match[1])
        .filter((args) => !args.includes('--release'));
      for (const product of ['tmt-ops', 'tmt-remote', 'tmt-colab']) {
        expect(
          debugBuilds.some((args) => new RegExp(`(?:^|\\s)-p\\s+${product}(?=\\s|$)`).test(args)),
          `${scope} native process fixtures must build ${product} in debug`
        ).toBe(true);
      }
      expect(
        debugBuilds.some((args) => /(?:^|\s)--example\s+runtime-caller-fixture(?=\s|$)/.test(args)),
        `${scope} native process fixtures must build the shared ancestry launcher`
      ).toBe(true);
      expect(
        debugBuilds.some((args) =>
          /(?:^|\s)-p\s+tmt-test-support\s+--example\s+recording-cli-fixture(?=\s|$)/.test(args)
        ),
        `${scope} native process fixtures must build the native recording driver`
      ).toBe(true);
    }
    expect(job('native-clippy')).toContain('shared-key: native-rust');
    expect(job('native-clippy')).toContain('save-if: false');
    const workspaceWorkflow = readFileSync(
      new URL('../../../.github/workflows/workspace-partitions.yml', import.meta.url),
      'utf8'
    );
    expect(workspaceWorkflow).toContain('shared-key: native-rust');
    expect(workspaceWorkflow).toContain(
      "save-if: ${{ github.ref == 'refs/heads/main' && inputs.save-cache }}"
    );
    expect(job('native-workspace-tests')).toContain(
      "save-cache: ${{ github.ref == 'refs/heads/main' && (needs.changes.outputs.verify == 'false' || needs.changes.outputs.main_full_admitted == 'true') }}"
    );
    // The process job seeds its existing dev/release builds in an isolated main-only cache.
    const processCache = job('native-process-tests');
    expect(processCache).toContain('shared-key: native-process-rust');
    expect(processCache).not.toContain('shared-key: native-rust');
    expect(processCache).not.toMatch(/cache-workspace-crates:|cache-on-failure:/);
    const save = /^ {10}save-if: (.+)$/m.exec(processCache)?.[1];
    expect(save).toBeDefined();
    const evaluateSave = new Function(
      'needs',
      'github',
      `return (${save!.replace(/^\$\{\{\s*|\s*\}\}$/g, '')})`
    );
    for (const ref of [
      'refs/heads/main',
      'refs/heads/feature',
      'refs/pull/1/merge',
      'refs/heads/gh-readonly-queue/main/pr-1',
      'refs/tags/v5.0.0-alpha.1',
    ]) {
      for (const verify of ['false', 'true', '']) {
        for (const scope of ['full', 'ops']) {
          expect(
            evaluateSave({ changes: { outputs: { verify, native_scope: scope } } }, { ref }),
            `${ref} verify=${verify} scope=${scope}`
          ).toBe(ref === 'refs/heads/main' && verify === 'false');
        }
      }
    }
    // The gate names the same eight jobs, in the order nativeGatePasses expects.
    const gate = job('native-install-gate');
    expect(gate).toContain('NATIVE_SCOPE: ${{ needs.changes.outputs.native_scope }}');
    expect(gate).toContain(
      'ci-scope.mjs gate-native "$MACOS_SELECTED" "$NATIVE_SCOPE" "$CONTRACT_RESULT" "$UNIT_RESULT" "$SHARD_1_RESULT" "$SHARD_2_RESULT" "$BUILD_RESULT" "$MATRIX_RESULT" "$MACOS_BUILD_RESULT" "$MACOS_MATRIX_RESULT"'
    );
    for (const [variable, jobName] of [
      ['CONTRACT_RESULT', 'native-rust'],
      ['UNIT_RESULT', 'unit-tests'],
      ['SHARD_1_RESULT', 'docker-e2e-shard-1'],
      ['SHARD_2_RESULT', 'docker-e2e-shard-2'],
      ['BUILD_RESULT', 'native-runtime-build'],
      ['MATRIX_RESULT', 'packed-native-install'],
      ['MACOS_BUILD_RESULT', 'native-runtime-build-macos'],
      ['MACOS_MATRIX_RESULT', 'packed-native-install-macos'],
    ]) {
      expect(gate).toContain(`${variable}: \${{ needs.${jobName}.result }}`);
    }
  });

  it('keeps the complete Colab harness separate from required gates and PR cache writes', () => {
    const workflow = readFileSync(
      new URL('../../../.github/workflows/colab-browser.yml', import.meta.url),
      'utf8'
    );
    const ci = readFileSync(new URL('../../../.github/workflows/ci.yml', import.meta.url), 'utf8');
    expect(ci).not.toContain('colab_harness');
    expect(ci).not.toContain('needs.colab-browser');
    expect(workflow).toContain('permissions: {}');
    expect(workflow).not.toContain('paths:');
    expect(workflow).not.toContain('pull_request_target:');
    expect(workflow).not.toContain('merge_group:');
    expect(workflow).not.toContain('continue-on-error');
    expect(workflow).not.toContain('\n  push:');
    expect(workflow).toContain('workflow_dispatch:');
    expect(workflow).toContain("cron: '23 5 * * 1'");
    expect(workflow).not.toMatch(/^concurrency:/m);
    expect(workflow).toContain('types: [opened, synchronize, reopened, labeled, unlabeled]');
    expect(ci).not.toContain('needs.colab-acceptance');
    const job = (name: string) => {
      const start = workflow.indexOf(`\n  ${name}:\n`);
      const next = workflow.slice(start + 1).search(/\n {2}[a-z0-9-]+:\n/);
      return workflow.slice(start, next < 0 ? undefined : start + 1 + next);
    };
    for (const name of ['changes', 'colab-browser']) {
      const body = job(name);
      expect(body).toMatch(/^ {4}concurrency:\n {6}group: colab-browser-/m);
      expect(body).toContain("cancel-in-progress: ${{ github.event_name == 'pull_request' }}");
    }
    expect(job('changes')).toContain(
      "github.event.action != 'labeled' && github.event.action != 'unlabeled'"
    );
    expect(job('colab-browser')).toContain('needs: changes');
    const app = job('colab-app');
    expect(app).toContain('needs: changes');
    expect(app).toMatch(/^ {4}concurrency:\n {6}group: colab-browser-app-/m);
    expect(app).toContain("cancel-in-progress: ${{ github.event_name == 'pull_request' }}");
    expect(app).toContain(
      "if: github.event_name == 'schedule' || github.event_name == 'workflow_dispatch' || needs.changes.outputs.colab_app == 'true'"
    );
    expect(app).toContain('permissions:\n      contents: read');
    expect(app).toContain('pnpm --filter @tmt/colab-app --fail-if-no-match test:browser');
    expect(app).toContain('playwright install --with-deps chromium');
    // Whole-file shards balance the one single-worker suite (#3618): none cancels another, and each keeps
    // its own concurrency group and evidence.
    expect(app).toContain('fail-fast: false');
    expect(app).toContain('shard: [1, 2, 3, 4, 5, 6]');
    expect(app).toContain(
      'shard_files="$(node scripts/colab-acceptance-shards.mjs ${{ matrix.shard }} 6 component)"'
    );
    expect(app).not.toContain('--shard=');
    expect(app).toContain('--reporter=dot,json');
    expect(app).toContain(
      'PLAYWRIGHT_JSON_OUTPUT_FILE: ${{ runner.temp }}/colab-app-results/results.json'
    );
    expect(app).toContain('-${{ matrix.shard }}\n      cancel-in-progress');
    expect(app).toContain('name: colab-app-results-${{ matrix.shard }}');
    expect(app).toContain('retention-days: 7');
    expect(app).not.toContain('COLAB_SERVE_EXECUTABLE');
    expect(app).not.toContain('cargo');
    expect(workflow).toContain('ci-scope.mjs "$BASE_SHA" "$HEAD_SHA" >> "$GITHUB_OUTPUT"');
    expect(workflow).toContain(
      "if: github.event_name == 'schedule' || github.event_name == 'workflow_dispatch' || needs.changes.outputs.colab_harness == 'true'"
    );
    expect(workflow).toContain('playwright install --with-deps chromium firefox webkit');
    expect(workflow).toContain('test:browser --engines chromium 2>&1 | tee');
    expect(workflow).toContain('--example browser_conformance --example browser_authority');
    expect(workflow).toContain('shell: bash');
    const linux = job('colab-browser');
    expect(linux).toContain('runs-on: ubuntu-latest');
    expect(linux).toContain('COLAB_EXPECT_REFUSED_ENGINES: webkit');
    expect(linux).toContain('test:browser --engines chromium,firefox,webkit 2>&1 | tee');
    expect(linux).toContain('pnpm --filter @tmt/colab-app --fail-if-no-match build');
    const macos = job('colab-webkit-macos');
    expect(macos).toContain('runs-on: macos-latest');
    expect(macos).not.toContain('COLAB_EXPECT_REFUSED_ENGINES');
    expect(macos).not.toContain('needs:');
    expect(macos).toContain('permissions:\n      contents: read');
    expect(macos).toContain('playwright install webkit');
    expect(macos).toContain('test:browser --engines webkit 2>&1 | tee');
    expect(macos).toContain('--example browser_conformance --example browser_authority');
    expect(macos).toContain('shared-key: native-rust\n          save-if: false');
    expect(macos).toContain('name: colab-webkit-macos-results');
    const admission = macos.match(/^ {4}if: (.+)$/m)?.[1];
    expect(admission).toBe(
      "github.event_name == 'schedule' || github.event_name == 'workflow_dispatch'"
    );
    const admitted = new Function('github', `return (${admission})`);
    for (const event_name of [
      'schedule',
      'workflow_dispatch',
      'pull_request',
      'merge_group',
      'push',
    ]) {
      expect(admitted({ event_name }), event_name).toBe(
        ['schedule', 'workflow_dispatch'].includes(event_name)
      );
    }
    expect(ci).not.toContain('colab-webkit-macos');
    expect(workflow).toContain('if: always()');
    expect(workflow).toContain('retention-days: 7');
    expect(workflow).toContain('actions/cache/restore@v4');
    expect(workflow).toContain(
      'colab-playwright-${{ runner.os }}-${{ runner.arch }}-${{ steps.playwright.outputs.version }}'
    );
    expect(workflow).toContain(
      "if: github.ref == 'refs/heads/main' && steps.browsers.outputs.cache-hit != 'true'"
    );
    expect(workflow).toContain('actions/cache/save@v4');
    expect(workflow).toContain('shared-key: native-rust\n          save-if: false');
    const acceptance = job('colab-acceptance');
    expect(acceptance).not.toContain('needs:');
    expect(acceptance).toContain(
      "github.event_name == 'schedule' || github.event_name == 'workflow_dispatch' ||"
    );
    expect(acceptance).toContain(
      "github.event_name == 'pull_request' && github.event.action != 'unlabeled'"
    );
    expect(acceptance).toContain(
      "contains(github.event.pull_request.labels.*.name, 'colab-acceptance')"
    );
    expect(acceptance).toContain(
      "github.event.action != 'labeled' || github.event.label.name == 'colab-acceptance'"
    );
    expect(acceptance).toMatch(/^ {4}concurrency:\n {6}group: colab-acceptance-/m);
    expect(acceptance).toContain('cancel-in-progress: false');
    expect(acceptance).toContain('runs-on: ubuntu-24.04');
    expect(acceptance).toContain('timeout-minutes: 30');
    expect(acceptance).toContain('permissions:\n      contents: read');
    expect(acceptance).not.toContain('CARGO_BUILD_JOBS:');
    expect(acceptance).toContain('cargo build --locked --manifest-path rust/Cargo.toml -j 2');
    expect(acceptance).toContain(
      'pnpm --filter @tmt/colab-app... --fail-if-no-match install --frozen-lockfile --ignore-scripts'
    );
    expect(acceptance).toContain(
      'uses: ./.github/actions/apt-install\n        with:\n          packages: tmux'
    );
    expect(acceptance).toContain('sh scripts/install-ci-rust.sh 1.97.0 --profile minimal');
    expect(acceptance).toContain('shared-key: native-rust\n          save-if: false');
    expect(acceptance).not.toContain('actions/cache/save');
    // A stale PR head may lack helpers added on main, including the always-run report step.
    // Default checkout tests the merge ref, where those workflow inputs are integrated.
    expect(acceptance).not.toMatch(/^\s*ref:/m);
    expect(acceptance).toContain('PR_HEAD_SHA: ${{ github.event.pull_request.head.sha }}');
    expect(acceptance).toContain(
      'printf \'pr-head: %s\\ntested-merge: %s\\n\' "$PR_HEAD_SHA" "$(git rev-parse HEAD)" > "$RUNNER_TEMP/colab-acceptance-results/tested-head.txt"'
    );
    expect(acceptance).toContain('persist-credentials: false');
    const appBuild = acceptance.indexOf('pnpm --filter @tmt/colab-app --fail-if-no-match build');
    const nativeBuild = acceptance.indexOf('cargo build --locked --manifest-path rust/Cargo.toml');
    expect(appBuild).toBeGreaterThan(0);
    expect(nativeBuild).toBeGreaterThan(appBuild);
    // One compile of the three executables; Remote's offline composer (#3048) and Core's
    // interpreted provider fixture (#3306) are each their own invocation, because an example
    // selected next to tmt-cli enables tmt-adapters' `test-isolation` dev-dependency feature
    // and the resulting `tmt` refuses to run in a world.
    expect(acceptance.match(/cargo build /g)).toHaveLength(3);
    expect(acceptance).toContain(
      'cargo build --locked --manifest-path rust/Cargo.toml -j 2 -p tmt-cli -p tmt-remote -p tmt-colab --bins\n'
    );
    expect(acceptance).toContain(
      'cargo build --locked --manifest-path rust/Cargo.toml -j 2 -p tmt-remote --example compose_hosting\n'
    );
    expect(acceptance).toContain(
      'cargo build --locked --manifest-path rust/Cargo.toml -j 2 -p tmt-adapters --example provider-script-fixture\n'
    );
    expect(acceptance).not.toMatch(/-p tmt-cli[^\n]*--example/);
    // The fixture is built before the shard runs and recorded with the other tested binaries.
    expect(acceptance.indexOf('--example provider-script-fixture')).toBeLessThan(
      acceptance.indexOf('test:acceptance --workers=1 --reporter=list,json')
    );
    expect(acceptance).toContain('rust/target/debug/examples/provider-script-fixture >');
    expect(acceptance).toContain('test:acceptance --workers=1 --reporter=list,json');
    // Two shards run the one single-worker suite (#2827): each builds its own binaries and world,
    // keeps its own concurrency group and evidence, and neither cancels the other.
    expect(acceptance).toContain('fail-fast: false');
    expect(acceptance).toContain('shard: [1, 2]');
    expect(acceptance).toContain(
      'shard_files="$(node scripts/colab-acceptance-shards.mjs ${{ matrix.shard }} 2)"'
    );
    expect(acceptance).toContain('if [ "${#files[@]}" -eq 0 ]; then');
    expect(acceptance).toContain('"${files[@]}"');
    expect(acceptance).not.toContain('--shard=');
    expect(acceptance).toContain('-${{ matrix.shard }}\n      # A label edit must never cancel');
    expect(acceptance).toContain('name: colab-acceptance-results-${{ matrix.shard }}');
    expect(acceptance).not.toContain('continue-on-error');
    expect(acceptance).not.toContain('|| true');
    const upload = acceptance.slice(
      acceptance.indexOf('      - name: Upload public acceptance evidence')
    );
    expect(upload).toContain('if: always()');
    expect(upload).toContain('path: ${{ runner.temp }}/colab-acceptance-results');
    expect(upload).toContain('retention-days: 7');
    expect(upload).not.toContain('colab-acceptance-private');
    expect(acceptance).toContain('node typescript/scripts/colab-acceptance-report.mjs');
    // The hosted inventory rides in the same single build (#2487): Remote's pinned Firebase client
    // and the hosted app are built first, then embedded by that one compile, never a second one.
    expect(acceptance).toContain(
      'TMT_COLAB_HOSTING_DIR: ${{ github.workspace }}/extensions/tmt-colab/typescript/app/dist-hosted'
    );
    expect(acceptance).toContain(
      'pnpm --filter @tmt/remote-client... --fail-if-no-match install --frozen-lockfile --ignore-scripts'
    );
    const firebaseBuild = acceptance.indexOf('exec vp build --config vite.firebase.config.ts');
    const hostedBuild = acceptance.indexOf(
      'pnpm --filter @tmt/colab-app --fail-if-no-match build:hosted'
    );
    expect(firebaseBuild).toBeGreaterThan(appBuild);
    expect(hostedBuild).toBeGreaterThan(appBuild);
    expect(nativeBuild).toBeGreaterThan(firebaseBuild);
    expect(nativeBuild).toBeGreaterThan(hostedBuild);
    expect(acceptance).toContain('hosted-assets.sha256');
    // A pull request that moves the Firebase pin, the hosted build or the spec runs only that one
    // spec in its own job; a label already runs it inside the full suite, so it never runs twice.
    expect(workflow).toContain("colab_hosted: ${{ steps.scope.outputs.colab_hosted || 'false' }}");
    const hosted = job('colab-hosted');
    expect(hosted).toContain('needs: changes');
    expect(hosted).toContain("needs.changes.outputs.colab_hosted == 'true'");
    expect(hosted).toContain(
      "!contains(github.event.pull_request.labels.*.name, 'colab-acceptance')"
    );
    expect(hosted).toContain("github.event_name == 'pull_request'");
    expect(hosted).toContain('permissions:\n      contents: read');
    expect(hosted).toContain('persist-credentials: false');
    expect(hosted).not.toContain('actions/cache/save');
    expect(hosted).toContain('shared-key: native-rust\n          save-if: false');
    expect(hosted).toContain('TMT_COLAB_HOSTING_DIR: ${{ github.workspace }}');
    expect(hosted.match(/cargo build /g)).toHaveLength(2);
    expect(hosted).toContain('-p tmt-remote --example compose_hosting');
    expect(hosted).not.toMatch(/-p tmt-cli[^\n]*--example/);
    expect(hosted).toContain('acceptance/hosted-bundle.spec.ts');
    expect(hosted).toContain('name: Colab hosted and native served specs (Chromium)');
    expect(hosted).toContain("COLAB_SERVE_REQUIRED: '1'");
    expect(hosted).toContain("COLAB_SERVE_EMBEDDED: '1'");
    expect(hosted).toContain("TMT_AGENT: '1'");
    expect(hosted).toContain('BROWSER: /usr/bin/true');
    expect(hosted).toContain('COLAB_SERVE_APP_DIR: ${{ runner.temp }}/colab-served-app');
    expect(hosted).toContain(
      'cp -R ../extensions/tmt-colab/typescript/app/dist "$COLAB_SERVE_APP_DIR"'
    );
    expect(hosted).toContain(
      'trap \'mv "$RUNNER_TEMP/colab-served-checkout" ../extensions/tmt-colab/typescript/app/dist\' EXIT'
    );
    expect(hosted).toContain('e2e/served.spec.ts --workers=1 --reporter=list,json');
    expect(hosted.indexOf('name: Run native served specs without skips')).toBeGreaterThan(
      hosted.indexOf('uses: ./.github/actions/hosted-reader-emulator')
    );
    expect(
      hosted
        .trim()
        .endsWith(
          'node scripts/colab-native-spec-report.mjs "$RUNNER_TEMP/colab-served-results/list.json" "$PLAYWRIGHT_JSON_OUTPUT_FILE"'
        )
    ).toBe(true);

    expect(hosted).not.toContain('colab-acceptance-shards');
    expect(hosted).not.toContain('continue-on-error');
  });
});

it.each(['Cargo.toml', 'src/store.rs', 'tests/state.rs'])(
  'keeps released Colab Rust %s in full workspace CI',
  (suffix) => {
    const files = ['extensions/tmt-colab/rust/tmt-colab/' + suffix];
    expect(explainCiSelection(files)).toMatchObject([
      { owner: 'tmt-colab', rule: 'colab-rust', native: true },
    ]);
    expect(selectNativeScope(files)).toBe('full');
    expect(
      isReleased(
        parseComponentMap(
          readFileSync(new URL('../../../.github/components.json', import.meta.url), 'utf8')
        ),
        'tmt-colab'
      )
    ).toBe(true);
  }
);

it('schedules architecture CI for the map, every declaring crate file and release script', () => {
  const root = fileURLToPath(new URL('../../../', import.meta.url));
  const map = parseComponentMap(readFileSync(`${root}.github/components.json`, 'utf8'));
  const workspace = readCargoWorkspace(root);
  const files: string[] = runPackedCommand('git', ['ls-files', '-z'], {
    cwd: root,
    env: process.env,
  })
    .split('\0')
    .filter(Boolean);
  const protectedPaths = ['.github/components.json', 'scripts/build-native-artifact.sh'];
  for (const component of map.components.filter(
    (c) => c.neverShippedPaths.length || c.generatedInputs.length
  )) {
    const crate = workspace.packages.find((p) => p.name === component.package)!;
    expect(crate).toBeDefined();
    protectedPaths.push(
      ...files.filter((path) => path.startsWith(`${crate.dir}/`)),
      `${crate.dir}/new-input.md`
    );
  }
  for (const path of protectedPaths) {
    expect(selectCiAreas([path], map).native, path).toBe(true);
    expect(selectNativeScope([path], map), path).toBe('full');
  }
  const workflow = readFileSync(`${root}.github/workflows/ci.yml`, 'utf8');
  const job = workflow.split('\n  native-workspace-tests:\n')[1].split(/\n {2}[a-z0-9-]+:\n/)[0];
  expect(job).toContain("if: needs.changes.outputs.native == 'true'");
  expect(job).toContain('scope: ${{ needs.changes.outputs.native_scope }}');
  const workspaceWorkflow = readFileSync(
    `${root}.github/workflows/workspace-partitions.yml`,
    'utf8'
  );
  expect(workspaceWorkflow).toContain("if: inputs.scope == 'full'");
  expect(workspaceWorkflow).toContain('cargo test --locked --workspace --doc');
  expect(workspaceWorkflow).toContain('workspace-nextest.mjs inventory');
});

it('pins and verifies Node before native-workspace-tests executes Rust tests', () => {
  const workflow = readFileSync(
    new URL('../../../.github/workflows/workspace-partitions.yml', import.meta.url),
    'utf8'
  );
  const job = workflow.split('\n  producer:\n')[1].split(/\n {2}[a-z0-9-]+:\n/)[0];
  expect(job).toContain(`uses: actions/setup-node@v4
        with:
          node-version: 22.23.2`);
  expect(job).toContain(`node --version
          test "$(node --version)" = v22.23.2`);
  expect(job.indexOf('actions/setup-node@v4')).toBeLessThan(job.indexOf('node --version'));
  expect(job.indexOf('node --version')).toBeLessThan(job.indexOf('cargo test'));
  expect(job).not.toContain('uses: ./.github/actions/setup-tooling');
});

it('keeps native E2E archive consumers restore-only and the sole writer main-only', () => {
  const ci = readFileSync(new URL('../../../.github/workflows/ci.yml', import.meta.url), 'utf8');
  const shard = ci.slice(ci.indexOf('  docker-e2e-shard-1:'), ci.indexOf('  docker-e2e-shard-2:'));
  expect(shard).toContain('uses: actions/cache/restore@v4');
  expect(shard).toContain('continue-on-error: true');
  expect(shard).not.toMatch(
    /actions\/cache\/save|restore-keys|cache-(?:from|to)|fail-on-cache-miss/
  );
  expect(shard).toContain("if: steps.e2e-admission.outputs.usable == 'true'");
  expect(shard).toContain('node typescript/scripts/e2e-dependency-cache.mjs admit');
  expect(shard).toContain("steps.e2e-admission.outputs.usable == 'true' && format");
  const seed = readFileSync(
    new URL('../../../.github/workflows/e2e-dependency-cache.yml', import.meta.url),
    'utf8'
  );
  expect(seed).toContain("if: github.event_name == 'push' && github.ref == 'refs/heads/main'");
  expect(seed).toContain('group: e2e-dependency-cache-${{ github.ref }}');
  expect(seed).toContain('cancel-in-progress: false');
  expect(seed).toContain('lookup-only: true');
  expect(seed).toContain('uses: actions/cache/save@v4');
  expect(seed).toContain('outputs: type=local,dest=${{ runner.temp }}/tmt-e2e-dependencies');
  expect(seed).toContain('provenance: false');
  expect(seed).toContain('sbom: false');
  expect(seed).not.toMatch(/pull_request|merge_group|workflow_dispatch|restore-keys|type=gha/);
  expect(seed).toContain("- '**/Cargo.toml'");
  expect(seed).toContain('- rust/Cargo.lock');
  expect(seed).toContain('node typescript/scripts/e2e-dependency-cache.mjs stamp');
});

it('enforces current PR Agent attribution for PRs and merge groups in Code quality', () => {
  const ci = readFileSync(new URL('../../../.github/workflows/ci.yml', import.meta.url), 'utf8');
  const quality = ci.split('\n  code-quality:\n')[1].split(/\n {2}[a-z0-9-]+:\n/)[0];
  expect(quality.indexOf('Check current PR Agent attribution')).toBeGreaterThan(
    quality.indexOf('Check conventional PR titles')
  );
  expect(quality).toContain('run: node typescript/scripts/pr-agent-check.mjs');
  expect(quality).not.toContain('pr-agent-check.mjs --');
  const feedback = readFileSync(
    new URL('../../../.github/workflows/pr-title.yml', import.meta.url),
    'utf8'
  );
  expect(feedback).toContain('types: [opened, edited, reopened, synchronize]');
  expect(feedback).toContain('name: Conventional PR title');
  expect(feedback).toContain('run: node typescript/scripts/pr-agent-check.mjs');
  expect(feedback).not.toContain('--report-only');
  expect(feedback).toContain('contents: read');
  expect(feedback).toContain('pull-requests: read');
  expect(ci).not.toContain('pull-requests: write');
});

describe('native Auth+Firestore emulator selection', () => {
  it.each([
    'extensions/tmt-remote/rust/tmt-remote/src/firebase_auth.rs',
    'extensions/tmt-remote/rust/tmt-remote/src/firebase_auth/emulator_tests.rs',
    'extensions/tmt-remote/rust/tmt-remote/src/native_sync/client.rs',
    'extensions/tmt-remote/rust/tmt-remote/src/sealed_sync.rs',
    'extensions/tmt-colab/contracts/vectors/deploy-declaration-v1.firestore.rules',
    'extensions/tmt-colab/contracts/vectors/deploy-declaration-v1.firestore.indexes.json',
    'extensions/tmt-remote/typescript/remote-client/test/emulator/native-qualify.mjs',
    'extensions/tmt-colab/scripts/cloud-sync-emulator.mjs',
    'extensions/tmt-colab/rust/tmt-colab/src/object_channel/cloud_sync.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/object_channel/cloud_sync/inbound.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/object_channel/cloud_sync/proof/world.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/object_channel/documents/originals.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/sync.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/fold.rs',
    '.github/workflows/ci.yml',
  ])('selects the real native client proof for %s', (file) => {
    expect(selectNativeSyncEmulator([file])).toBe(true);
    expect(selectCiAreas([file]).native).toBe(true);
    expect(selectNativeScope([file])).toBe('full');
  });
  it.each([
    'site/src/chapters/start.mdx',
    'extensions/tmt-remote/rust/tmt-remote/src/serve.rs',
    'extensions/tmt-remote/rust/tmt-remote/src/native_sync-other/client.rs',
    'extensions/tmt-colab/contracts/vectors/deploy-declaration-v1-other.json',
    'extensions/tmt-colab/rust/tmt-colab/src/object_channel/cloud_sync-other/inbound.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/object_channel/attach.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/sync/wire.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/syncing.rs',
    'extensions/tmt-colab/rust/tmt-colab/src/store.rs',
  ])('does not select unrelated %s', (file) => {
    expect(selectNativeSyncEmulator([file])).toBe(false);
  });
  it('runs one selected ignored test in the existing workspace job, with no new dependency edge', () => {
    const ci = readFileSync(
      new URL('../../../.github/workflows/workspace-partitions.yml', import.meta.url),
      'utf8'
    );
    const job = ci.split('\n  producer:\n')[1].split('\n  consumers:')[0];
    expect(job).toContain('runs-on: ubuntu-latest');
    expect(job).toContain("inputs.scope == 'full' && inputs.native-sync");
    expect(job).toContain('firebase-tools@15.29.0');
    expect(job).toContain('timeout-minutes: 8');
    expect(job).toContain('test/emulator/native-qualify.mjs');
    const runner = readFileSync(
      new URL(
        '../../../extensions/tmt-remote/typescript/remote-client/test/emulator/native-qualify.mjs',
        import.meta.url
      ),
      'utf8'
    );
    expect(runner).toMatch(/'--ignored',\s*'--exact'/);
    expect(runner).toContain('runInPrivateWorld');
    expect(runner).toContain("'127.0.0.1'");
    expect(runner).toContain('1 passed; 0 failed; 0 ignored;');
    expect(runner).not.toContain('docker');
  });
  it('runs the Colab cloud sync proof after the native one, from a script that names exactly its scenarios', () => {
    const ci = readFileSync(
      new URL('../../../.github/workflows/workspace-partitions.yml', import.meta.url),
      'utf8'
    );
    const job = ci.split('\n  producer:\n')[1].split('\n  consumers:')[0];
    expect(job.indexOf('native-qualify.mjs')).toBeLessThan(
      job.indexOf('extensions/tmt-colab/scripts/cloud-sync-emulator.mjs')
    );
    const runner = readFileSync(
      new URL('../../../extensions/tmt-colab/scripts/cloud-sync-emulator.mjs', import.meta.url),
      'utf8'
    );
    expect(runner).toContain('runInPrivateWorld');
    expect(runner).toContain("'--ignored'");
    expect(runner).toContain('const scenarios = 5;');
    expect(runner).toContain('passed; 0 failed; 0 ignored;');
    expect(runner).toContain("host: '127.0.0.1'");
    expect(runner).not.toContain('docker');
  });
});

describe('hosted producer/reader emulator workflow wiring', () => {
  const workflow = readFileSync(
    new URL('../../../.github/workflows/colab-browser.yml', import.meta.url),
    'utf8'
  );
  const job = (name: string) =>
    workflow.split(`  ${name}:\n`)[1].split(/\n  [a-z][a-z0-9-]*:\n/)[0];

  it('admits selected PRs through either predicate without duplicating label acceptance', () => {
    expect(job('changes')).toContain(
      "colab_hosted_emulator: ${{ steps.scope.outputs.colab_hosted_emulator || 'false' }}"
    );
    const hosted = job('colab-hosted');
    const condition = hosted.match(/^    if: >-\n([\s\S]*?)\n    concurrency:/m)?.[1];
    expect(condition).toBeDefined();
    const admit = new Function(
      'github',
      'needs',
      'contains',
      `return (${condition?.replace('github.event.pull_request.labels.*.name', 'github.event.pull_request.labels.map(label => label.name)')})`
    );
    for (const event_name of [
      'pull_request',
      'merge_group',
      'push',
      'schedule',
      'workflow_dispatch',
    ]) {
      for (const oldSelection of ['true', 'false'])
        for (const emulator of ['true', 'false'])
          for (const labelled of [true, false]) {
            const github = {
              event_name,
              event: { pull_request: { labels: labelled ? [{ name: 'colab-acceptance' }] : [] } },
            };
            const needs = {
              changes: { outputs: { colab_hosted: oldSelection, colab_hosted_emulator: emulator } },
            };
            expect(
              admit(github, needs, (values: string[], value: string) => values.includes(value))
            ).toBe(
              event_name === 'pull_request' &&
                !labelled &&
                (oldSelection === 'true' || emulator === 'true')
            );
          }
    }
    expect(hosted).toContain(
      "if: needs.changes.outputs.colab_hosted_emulator == 'true'\n        uses: ./.github/actions/hosted-reader-emulator"
    );
    expect(hosted.indexOf('uses: ./.github/actions/hosted-reader-emulator')).toBeGreaterThan(
      hosted.indexOf('acceptance/hosted-bundle.spec.ts')
    );
    expect(hosted).toContain('timeout-minutes: 20');
    expect(hosted).toContain('permissions:\n      contents: read');
  });

  it('qualifies once after ordinary acceptance regardless of its result unless cancelled', () => {
    const acceptance = job('colab-acceptance');
    expect(acceptance.match(/uses: \.\/.github\/actions\/hosted-reader-emulator/g)).toHaveLength(1);
    expect(acceptance).toContain(
      'if: ${{ !cancelled() && matrix.shard == 1 }}\n        uses: ./.github/actions/hosted-reader-emulator'
    );
    const qualifier = acceptance.indexOf('uses: ./.github/actions/hosted-reader-emulator');
    expect(qualifier).toBeGreaterThan(
      acceptance.indexOf('name: Record the tested source and exact build hashes')
    );
    expect(qualifier).toBeGreaterThan(
      acceptance.indexOf('pnpm --filter @tmt/colab-app --fail-if-no-match test:acceptance')
    );
    expect(acceptance).toContain('timeout-minutes: 30');
    expect(acceptance).toContain('permissions:\n      contents: read');
  });

  it('reuses the owner wrapper with pinned Java/Firebase and no retry or credentials', () => {
    const action = readFileSync(
      new URL('../../../.github/actions/hosted-reader-emulator/action.yml', import.meta.url),
      'utf8'
    );
    expect(action).toContain('using: composite');
    expect(action).toContain('test -n "$JAVA_HOME_21_X64"');
    expect(action).toContain('echo "JAVA_HOME=$JAVA_HOME_21_X64" >> "$GITHUB_ENV"');
    expect(action).toContain('echo "$JAVA_HOME_21_X64/bin" >> "$GITHUB_PATH"');
    expect(action).toContain('npm install --global firebase-tools@15.29.0');
    expect(action).toContain('firebase setup:emulators:firestore');
    expect(action).toContain('node extensions/tmt-colab/scripts/hosted-reader-emulator.mjs');
    const diagnostics = action.split('    - name: Preserve failed hosted reader diagnostics\n')[1];
    expect(diagnostics).toContain('if: failure()');
    expect(diagnostics).toContain('uses: actions/upload-artifact@v4');
    expect(diagnostics).toContain('path: extensions/tmt-colab/typescript/app/test-results/');
    expect(diagnostics).toContain('retention-days: 3');
    expect(diagnostics).toContain(
      '${{ github.run_id }}-${{ github.run_attempt }}-${{ github.job }}'
    );
    expect(action.indexOf('Preserve failed hosted reader diagnostics')).toBeGreaterThan(
      action.indexOf('node extensions/tmt-colab/scripts/hosted-reader-emulator.mjs')
    );
    expect(action.indexOf('firebase setup:emulators:firestore')).toBeLessThan(
      action.indexOf('node extensions/tmt-colab/scripts/hosted-reader-emulator.mjs')
    );
    expect(action).not.toMatch(
      /continue-on-error|\|\| true|secrets\.|GH_TOKEN|GITHUB_TOKEN|docker|retry-command/
    );
  });
});
