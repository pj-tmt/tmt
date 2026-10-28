import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vite-plus/test';
import { CAPTURE_MAX_UNREAD, captureMeterScript } from '../e2e/capture-meter.js';

let root: string;

beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'capture-meter-'));
});

afterEach(() => fs.rmSync(root, { recursive: true, force: true }));

const unchanged = '{"attempted":1,"published":0,"skipped":0,"reasons":{"unchanged":1}}';
const published = '{"attempted":1,"published":1,"skipped":0}';

/** A stand-in for `tmt api`: it logs a topology read, as the tmux wrapper would, then answers. */
function stubApi(body: string): string[] {
  const file = path.join(root, 'api.sh');
  fs.writeFileSync(file, `#!/bin/sh\n${body}\n`, { mode: 0o755 });
  return [file];
}

const reads = (name: string) =>
  `printf 'list-panes\\t%s\\n' ${name} >> ${path.join(root, 'trace')}`;

function runMeter(mode: 'samples' | 'changed', seconds: number, command: string[], samples = 3) {
  fs.writeFileSync(path.join(root, 'trace'), '');
  const script = path.join(root, 'meter.py');
  fs.writeFileSync(
    script,
    captureMeterScript({
      directory: root,
      result: path.join(root, 'result.json'),
      trace: path.join(root, 'trace'),
      globalDir: path.join(root, 'home'),
      command,
      samples,
    })
  );
  const started = Date.now();
  const run = spawnSync('python3', [script, mode, String(seconds)], { encoding: 'utf8' });
  return { run, elapsedMs: Date.now() - started };
}

const countCall = () => `echo call >> ${path.join(root, 'calls')}`;
const callCount = () => fs.readFileSync(path.join(root, 'calls'), 'utf8').trim().split('\n').length;
const read = (name: string) => fs.readFileSync(path.join(root, name), 'utf8');
const exists = (name: string) => fs.existsSync(path.join(root, name));

describe('capture meter', () => {
  it('measures every sample with one topology read each and leaves progress behind', () => {
    const { run } = runMeter('samples', 5, stubApi(`${reads('a')}\necho '${unchanged}'`));
    expect(run.status, run.stderr).toBe(0);
    const result = JSON.parse(read('result.json'));
    expect(result.samples).toHaveLength(3);
    expect(result.unreadAttempts).toBe(0);
    expect(JSON.parse(read('result.json.progress'))).toMatchObject({ mode: 'samples', index: 2 });
    expect(exists('result.json.failure')).toBe(false);
  });

  it('re-measures a bounded number of attempts that failed before reading the topology', () => {
    const counter = path.join(root, 'calls');
    const body = [
      `n=$(cat ${counter} 2>/dev/null || echo 0); echo $((n+1)) > ${counter}`,
      `if [ "$n" -lt ${CAPTURE_MAX_UNREAD} ]; then echo '{"attempted":1,"published":0,"skipped":0,"reasons":{"busy":1}}'; exit 0; fi`,
      reads('a'),
      `echo '${unchanged}'`,
    ].join('\n');
    const { run } = runMeter('samples', 5, stubApi(body));
    expect(run.status, run.stderr).toBe(0);
    const result = JSON.parse(read('result.json'));
    expect(result.unreadAttempts).toBe(CAPTURE_MAX_UNREAD);
    expect(result.unreadReasons).toEqual([{ busy: 1 }, { busy: 1 }]);
    expect(result.samples).toHaveLength(3);
  });

  it('fails a sample that took two topology reads and names the count', () => {
    const { run } = runMeter(
      'samples',
      5,
      stubApi(`${reads('a')}\n${reads('b')}\necho '${unchanged}'`)
    );
    expect(run.status).toBe(1);
    expect(read('result.json.failure')).toContain(
      'sample 0 took 2 topology reads, expected exactly 1'
    );
    expect(exists('result.json')).toBe(false);
  });

  it('does not count an unchanged answer that made no read as a retry', () => {
    const { run } = runMeter('samples', 5, stubApi(`${countCall()}\necho '${unchanged}'`));
    expect(run.status).toBe(1);
    expect(read('result.json.failure')).toContain(
      "sample 0 made no topology read but reported {'unchanged': 1}"
    );
    expect(callCount()).toBe(1);
  });

  it('fails a sample call that publishes', () => {
    const published = '{"attempted":1,"published":1,"skipped":0,"reasons":{"unchanged":1}}';
    const { run } = runMeter('samples', 5, stubApi(`${reads('a')}\necho '${published}'`));
    expect(run.status).toBe(1);
    expect(read('result.json.failure')).toContain('a sample call must not publish or skip');
  });

  it('stops after the bounded retries when no attempt ever reads, naming the reasons', () => {
    const busy = '{"attempted":1,"published":0,"skipped":0,"reasons":{"busy":1}}';
    const { run, elapsedMs } = runMeter('samples', 30, stubApi(`${countCall()}\necho '${busy}'`));
    expect(run.status).toBe(1);
    expect(callCount()).toBe(CAPTURE_MAX_UNREAD + 1);
    expect(read('result.json.failure')).toContain('sample 0 took 0 topology reads');
    expect(read('result.json.failure')).toContain("[{'busy': 1}, {'busy': 1}]");
    expect(elapsedMs).toBeLessThan(10_000);
  });

  it('names a failing capture call in a file instead of only in the pane', () => {
    const { run } = runMeter('samples', 5, stubApi('echo broken >&2; exit 7'));
    expect(run.status).toBe(1);
    expect(read('result.json.failure')).toContain('AssertionError');
    expect(read('result.json.failure')).toContain('7');
    expect(exists('result.json')).toBe(false);
  });

  it('ends a stalled capture call inside its allowance and says so', () => {
    const { run, elapsedMs } = runMeter('samples', 0.4, stubApi('exec sleep 30'));
    expect(run.status).toBe(1);
    expect(read('result.json.failure')).toContain('TimeoutExpired');
    expect(elapsedMs).toBeLessThan(3_000);
  });

  it('makes no capture call once the allowance is spent', () => {
    const marker = path.join(root, 'called');
    const { run } = runMeter('samples', 0, stubApi(`touch ${marker}\necho '${unchanged}'`));
    expect(run.status).toBe(1);
    expect(read('result.json.failure')).toContain('the stage allowance was spent');
    expect(fs.existsSync(marker)).toBe(false);
  });

  it('publishes the changed-layout answer for the probe to read', () => {
    const { run } = runMeter('changed', 5, stubApi(`echo '${published}'`));
    expect(run.status, run.stderr).toBe(0);
    expect(JSON.parse(read('result.json.positive'))).toEqual({
      attempted: 1,
      published: 1,
      skipped: 0,
    });
  });

  it('publishes a changed-layout failure as an answer, so the probe stops waiting at once', () => {
    const { run } = runMeter('changed', 5, stubApi(`echo '${unchanged}'`));
    expect(run.status).toBe(1);
    const outcome = JSON.parse(read('result.json.positive'));
    expect(outcome.error).toContain('AssertionError');
    expect(read('result.json.failure')).toBe(outcome.error);
  });
});
