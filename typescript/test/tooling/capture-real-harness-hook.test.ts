import { mkdtempSync, readFileSync, readdirSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { afterEach, expect, it } from 'vite-plus/test';
import { forwardCapturedHook } from '../support/capture-real-harness-hook.mjs';
import { writeExecutable } from '../support/executable-fixture.mjs';

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});
function fixture() {
  const root = mkdtempSync(path.join(tmpdir(), 'tmt-synthetic-hook-'));
  roots.push(root);
  const executable = path.join(root, 'fake-tmt');
  const received = path.join(root, 'received');
  writeExecutable(
    executable,
    `#!${process.execPath}\nimport {readFileSync,writeFileSync} from 'node:fs';\nif(process.argv[2]==='--version'){writeFileSync(${JSON.stringify(received)},'public');process.exit(6); }\nwriteFileSync(${JSON.stringify(received)},readFileSync(0));process.exit(7);\n`
  );
  return { root, executable, received, directory: path.join(root, 'captures') };
}
async function* chunks(bytes: Buffer) {
  yield bytes.subarray(0, 5);
  yield bytes.subarray(5);
}
it('forwards exact synthetic bytes and the child status, with private bounded capture files', async () => {
  const f = fixture();
  const bytes = Buffer.from(
    '{"hook_event_name":"SessionStart","session_id":"synthetic","text":"界"}\n'
  );
  expect(
    await forwardCapturedHook(f.executable, ['__hook', 'codex'], f.directory, chunks(bytes))
  ).toBe(7);
  expect(readFileSync(f.received)).toEqual(bytes);
  expect(readFileSync(path.join(f.directory, '0.json'))).toEqual(bytes);
  expect(statSync(path.join(f.directory, '0.json')).mode & 0o777).toBe(0o600);
});
it('refuses oversized input before capture or child execution', async () => {
  const f = fixture();
  await expect(
    forwardCapturedHook(f.executable, ['__hook', 'codex'], f.directory, chunks(Buffer.alloc(65537)))
  ).rejects.toThrow('HARNESS_HOOK_CAPTURE_BYTES');
  expect(readdirSync(f.root)).toEqual(['fake-tmt']);
});
it('never overwrites the eight retained inputs and refuses a ninth invocation', async () => {
  const f = fixture();
  for (let i = 0; i < 8; i++)
    await forwardCapturedHook(
      f.executable,
      ['__hook', 'codex'],
      f.directory,
      chunks(Buffer.from(String(i)))
    );
  await expect(
    forwardCapturedHook(
      f.executable,
      ['__hook', 'codex'],
      f.directory,
      chunks(Buffer.from('ninth'))
    )
  ).rejects.toThrow('HARNESS_HOOK_CAPTURE_COUNT');
  expect(readFileSync(path.join(f.directory, '0.json'), 'utf8')).toBe('0');
  expect(readFileSync(f.received, 'utf8')).toBe('7');
});
it('refuses the recorder CLI outside the explicitly marked advisory container', () => {
  const result = spawnSync(
    process.execPath,
    [fileURLToPath(new URL('../support/capture-real-harness-hook.mjs', import.meta.url))],
    { encoding: 'utf8', env: { ...process.env, TMT_REAL_HARNESS: '' }, timeout: 5000 }
  );
  expect(result.status).not.toBe(0);
  expect(result.stderr).toContain('HARNESS_CONTAINER_REQUIRED');
});

it('passes public launcher calls through without reading or recording stdin', async () => {
  const f = fixture();
  async function* unexpected(): AsyncGenerator<Uint8Array> {
    throw new Error('must not read');
  }
  expect(await forwardCapturedHook(f.executable, ['--version'], f.directory, unexpected())).toBe(6);
  expect(readFileSync(f.received, 'utf8')).toBe('public');
  expect(readdirSync(f.root).sort()).toEqual(['fake-tmt', 'received']);
});
