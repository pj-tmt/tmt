import { once } from 'node:events';
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';

const helper = new URL('../../scripts/colab-acceptance-run.mjs', import.meta.url).href;

function runFixture(source: string, budgetMs = 3000, graceMs = 500) {
  const root = mkdtempSync(path.join(os.tmpdir(), 'tmt-acceptance-budget-'));
  try {
    const verdict = path.join(root, 'execution.json');
    const flushed = path.join(root, 'flushed');
    const parent = `
      import { runAcceptance } from ${JSON.stringify(helper)};
      process.exitCode = await runAcceptance({
        command: process.execPath, args: ['-e', ${JSON.stringify(source)}, ${JSON.stringify(flushed)}],
        deadlineMs: Date.now() + ${budgetMs}, graceMs: ${graceMs},
        verdictPath: ${JSON.stringify(verdict)}
      });
    `;
    const child = spawnSync(process.execPath, ['--input-type=module', '-e', parent], {
      encoding: 'utf8',
      timeout: 15_000,
    });
    expect(child.error).toBeUndefined();
    expect(child.signal).toBeNull();
    return {
      code: child.status,
      verdict: JSON.parse(readFileSync(verdict, 'utf8')),
      flushed: existsSync(flushed) ? readFileSync(flushed, 'utf8') : null,
      output: child.stdout + child.stderr,
    };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

describe('acceptance process owner', () => {
  it('preserves normal success and ordinary failure without deadline attribution', () => {
    expect(runFixture('process.exit(0)').verdict).toEqual({ state: 'completed', exitCode: 0 });
    const failed = runFixture('process.exit(7)');
    expect(failed.code).toBe(7);
    expect(failed.verdict).toEqual({ state: 'completed', exitCode: 7 });
  });

  it('never launches a child after the absolute deadline', () => {
    const result = runFixture("require('fs').writeFileSync(process.argv[1], 'launched')", -1);
    expect(result.code).toBe(124);
    expect(result.flushed).toBeNull();
    expect(result.verdict).toEqual({ state: 'deadline', forced: false, launched: false });
  });

  it('allows SIGINT to flush evidence but keeps a deadline failure even if the child exits zero', () => {
    const result = runFixture(`
      process.on('SIGINT', () => {
        require('fs').writeFileSync(process.argv[1], 'flushed');
        process.exit(0);
      });
      setInterval(() => {}, 1000);
    `);
    expect(result.code).toBe(124);
    expect(result.flushed).toBe('flushed');
    expect(result.verdict).toEqual({ state: 'deadline', forced: false, launched: true });
  });

  it('forwards a job signal through the same graceful path', () => {
    const result = runFixture(`
      process.on('SIGINT', () => {
        require('fs').writeFileSync(process.argv[1], 'signal-flushed');
        process.exit(0);
      });
      process.kill(process.ppid, 'SIGTERM');
      setInterval(() => {}, 1000);
    `);
    expect(result.code).toBe(130);
    expect(result.flushed).toBe('signal-flushed');
    expect(result.verdict).toEqual({ state: 'interrupted', forced: false, launched: true });
  });

  it('bounds a non-flushing child with SIGKILL of its owned group', () => {
    const result = runFixture(`process.on('SIGINT', () => {}); setInterval(() => {}, 1000);`);
    expect(result.code).toBe(124);
    expect(result.verdict).toEqual({ state: 'deadline', forced: true, launched: true });
    expect(result.flushed).toBeNull();
  });

  it('does not signal an unrelated process when its owned group is forced down', async () => {
    const unrelated = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], {
      stdio: 'ignore',
    });
    const exited = once(unrelated, 'exit');
    try {
      const result = runFixture(`process.on('SIGINT', () => {}); setInterval(() => {}, 1000);`);
      expect(result.verdict.forced).toBe(true);
      expect(unrelated.pid).toBeDefined();
      expect(() => process.kill(unrelated.pid!, 0)).not.toThrow();
    } finally {
      unrelated.kill('SIGTERM');
      await exited;
    }
  });

  it('keeps the grace bound when the group leader exits before its descendant', () => {
    const result = runFixture(`
      const child = require('child_process').spawn(process.execPath, ['-e',
        "process.on('SIGINT', () => {}); process.send('ready'); setInterval(() => {}, 1000);"
      ], { stdio: ['ignore', 'inherit', 'inherit', 'ipc'] });
      child.once('message', () => {
        process.on('SIGINT', () => process.exit(0));
      });
      setInterval(() => {}, 1000);
    `);
    expect(result.code).toBe(124);
    expect(result.verdict).toEqual({ state: 'deadline', forced: true, launched: true });
  });

  it('uses the job timestamp rather than granting a fresh budget after setup', () => {
    const source = `import { acceptanceDeadline } from ${JSON.stringify(helper)};
      console.log(JSON.stringify([acceptanceDeadline(1000, 1), acceptanceDeadline(1000, 2)]));`;
    const result = spawnSync(process.execPath, ['--input-type=module', '-e', source], {
      encoding: 'utf8',
    });
    expect(result.status).toBe(0);
    expect(JSON.parse(result.stdout)).toEqual([2_560_000, 2_740_000]);
  });

  it('publishes an incomplete verdict through the workflow CLI paths when no JSON flushed', () => {
    const root = mkdtempSync(path.join(os.tmpdir(), 'tmt-acceptance-cli-'));
    try {
      const env = {
        ...process.env,
        RUNNER_TEMP: root,
        TMT_ACCEPTANCE_STARTED_AT: '1',
        PLAYWRIGHT_JSON_OUTPUT_FILE: path.join(root, 'private-missing.json'),
      };
      const child = spawnSync(
        process.execPath,
        [
          fileURLToPath(helper),
          '1',
          process.execPath,
          '-e',
          'throw new Error("PRIVATE_NEVER_LAUNCHED")',
        ],
        { env, encoding: 'utf8' }
      );
      expect(child.status).toBe(124);
      expect(child.stdout + child.stderr).not.toContain('PRIVATE_NEVER_LAUNCHED');
      const report = spawnSync(
        process.execPath,
        [fileURLToPath(new URL('../../scripts/colab-acceptance-report.mjs', import.meta.url))],
        { cwd: root, env, encoding: 'utf8' }
      );
      expect(report.status).toBe(1);
      const evidence = readFileSync(
        path.join(root, 'colab-acceptance-results/outcomes.json'),
        'utf8'
      );
      expect(JSON.parse(evidence)).toEqual({
        run: { status: 'incomplete', reason: 'deadline' },
        tests: [],
      });
      expect(evidence + report.stdout + report.stderr).not.toContain(root);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it('records startup refusal without publishing the error payload', () => {
    const root = mkdtempSync(path.join(os.tmpdir(), 'tmt-acceptance-startup-'));
    try {
      const verdictPath = path.join(root, 'execution.json');
      const source = `import { runAcceptance } from ${JSON.stringify(helper)};
        process.exitCode = await runAcceptance({command: '/PRIVATE_SENTINEL/missing', args: [],
          deadlineMs: Date.now()+3000, graceMs: 50, verdictPath: ${JSON.stringify(verdictPath)}});`;
      const child = spawnSync(process.execPath, ['--input-type=module', '-e', source], {
        encoding: 'utf8',
      });
      expect(child.status).toBe(1);
      expect(JSON.parse(readFileSync(verdictPath, 'utf8'))).toEqual({ state: 'startup-failed' });
      expect(child.stdout + child.stderr).not.toContain('PRIVATE_SENTINEL');
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
