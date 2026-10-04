import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import { writeExecutable } from '../support/executable-fixture.mjs';
import {
  captureStallDiagnostics,
  warmReleaseVersion,
} from '../../scripts/warm-release-version.mjs';

const dirs: string[] = [];
afterEach(() => {
  for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

/** A stand-in helper: the first launch sleeps `firstMs` (the stall), later launches are instant. */
function helper(options: { firstMs?: number; exit?: number; secondMs?: number } = {}) {
  const dir = mkdtempSync(join(tmpdir(), 'tmt-warm-test-'));
  dirs.push(dir);
  const marker = join(dir, 'launched');
  const tool = join(dir, 'release-version');
  writeExecutable(
    tool,
    `#!${process.execPath}
const fs = require('node:fs');
const first = !fs.existsSync(${JSON.stringify(marker)});
fs.writeFileSync(${JSON.stringify(marker)}, '');
process.stdin.resume();
process.stdin.on('end', () => {
  setTimeout(() => {
    if (${options.exit ?? 0}) { process.stderr.write('helper rejected the input'); process.exit(${options.exit ?? 0}); }
    process.stdout.write('{}');
  }, first ? ${options.firstMs ?? 0} : ${options.secondMs ?? 0});
});
`,
    0o700
  );
  return tool;
}
const quick = { waitMs: 3000, tickMs: 40, diagnoseAtMs: 120, secondBoundMs: 1500 };

describe('release-version first-exec warm step', () => {
  it('waits out a first exec that stalls past the diagnose point, then proves a fast second exec', async () => {
    const log = vi.fn();
    const diagnose = vi.fn(() => 'ps and sample output');
    const result = await warmReleaseVersion({
      ...quick,
      tool: helper({ firstMs: 400 }),
      log,
      diagnose,
    });
    expect(result.firstMs).toBeGreaterThanOrEqual(350);
    expect(result.secondMs).toBeLessThan(result.firstMs);
    // Diagnostics are captured once, while the process is still alive, and the first exec is not killed.
    expect(diagnose).toHaveBeenCalledTimes(1);
    expect(diagnose).toHaveBeenCalledWith(expect.any(Number));
    const lines = log.mock.calls.map(([line]) => String(line));
    expect(lines.some((line) => line.includes('still running'))).toBe(true);
    expect(lines.some((line) => line.includes('ps and sample output'))).toBe(true);
    expect(lines.filter((line) => /first exec took/.test(line))).toHaveLength(1);
    expect(lines.filter((line) => /second exec took/.test(line))).toHaveLength(1);
  });
  it('stays quiet and skips diagnostics for a prompt first exec', async () => {
    const log = vi.fn();
    const diagnose = vi.fn();
    // Thresholds sit far above Node's own startup so only a real stall could cross them.
    await warmReleaseVersion({
      ...quick,
      waitMs: 8000,
      tickMs: 1000,
      diagnoseAtMs: 4000,
      tool: helper(),
      log,
      diagnose,
    });
    expect(diagnose).not.toHaveBeenCalled();
    expect(log.mock.calls.map(([line]) => String(line)).join('\n')).not.toContain('still running');
  });
  it('kills and fails when the first exec outlives the whole wait', async () => {
    const diagnose = vi.fn(() => 'evidence');
    await expect(
      warmReleaseVersion({
        waitMs: 300,
        tickMs: 40,
        diagnoseAtMs: 100,
        secondBoundMs: 500,
        tool: helper({ firstMs: 5000 }),
        log: () => {},
        diagnose,
      })
    ).rejects.toThrow('did not finish within 0.3s');
    expect(diagnose).toHaveBeenCalledTimes(1);
  });
  it('fails loudly on a helper that exits non-zero or cannot launch', async () => {
    await expect(
      warmReleaseVersion({ ...quick, tool: helper({ exit: 3 }), log: () => {} })
    ).rejects.toThrow('first exec failed: helper rejected the input');
    await expect(
      warmReleaseVersion({
        ...quick,
        tool: join(tmpdir(), 'tmt-missing-release-version'),
        log: () => {},
      })
    ).rejects.toThrow('ENOENT');
  });
  it('fails when the second exec is not quick, since the stall would then be real work', async () => {
    await expect(
      warmReleaseVersion({
        ...quick,
        secondBoundMs: 150,
        tool: helper({ secondMs: 3000 }),
        log: () => {},
      })
    ).rejects.toThrow('second exec failed');
  });
});

describe('stall diagnostics', () => {
  it('captures ps everywhere and sample only on macOS, and never throws', () => {
    const run = vi.fn((command: string, _args: string[]) => ({
      stdout: `${command} output`,
      stderr: '',
    }));
    const linux = captureStallDiagnostics(42, { platform: 'linux', run });
    expect(run.mock.calls.map(([command]) => command)).toEqual(['ps']);
    expect(linux).toContain('ps output');
    run.mockClear();
    const mac = captureStallDiagnostics(42, { platform: 'darwin', run });
    expect(run.mock.calls.map(([command, args]) => [command, args[0]])).toEqual([
      ['ps', '-arxo'],
      ['sample', '42'],
    ]);
    expect(mac).toContain('sample output');
    const failing = captureStallDiagnostics(42, {
      platform: 'darwin',
      run: () => ({ error: new Error('sample denied') }),
    });
    expect(failing).toContain('sample denied');
  });
});
