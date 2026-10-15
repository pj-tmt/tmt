import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import { publishDigestStep } from '../e2e/claude-digest-step.js';

const roots: string[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  for (const root of roots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

describe('Claude Digest command publication', () => {
  it('hides an incomplete write and publishes one complete command for one consumption', async () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'claude-digest-step-'));
    roots.push(root);
    const log = path.join(root, 'worker.log');
    const command = `${log}.digest-step`;
    const step = { event: 'published-stop', session: 'exact-session', active: true };
    const write = fs.promises.writeFile.bind(fs.promises);
    let release!: () => void;
    const blocked = new Promise<void>((resolve) => {
      release = resolve;
    });
    let staged!: () => void;
    const partial = new Promise<void>((resolve) => {
      staged = resolve;
    });
    const spy = vi
      .spyOn(fs.promises, 'writeFile')
      .mockImplementationOnce(async (file, data, options) => {
        const bytes = String(data);
        await write(file, bytes.slice(0, 8), options);
        staged();
        await blocked;
        await fs.promises.appendFile(file, bytes.slice(8));
      });
    const publication = publishDigestStep(log, step);
    const consumed: unknown[] = [];
    // Independent reader follows the mock's strict exists/read/parse/remove protocol.
    const consume = () => {
      if (!fs.existsSync(command)) return;
      consumed.push(JSON.parse(fs.readFileSync(command, 'utf8')));
      fs.rmSync(command);
    };
    try {
      await partial;
      expect(fs.existsSync(command), 'an incomplete command is not published').toBe(false);
      consume();
      expect(consumed).toEqual([]);
    } finally {
      release();
      try {
        await publication;
      } finally {
        spy.mockRestore();
      }
    }
    consume();
    consume();
    expect(consumed).toEqual([step]);
    expect(fs.readdirSync(root), 'no staging files survive publication').toEqual([]);
  });

  it('removes staging bytes and leaves the command unpublished when the write fails', async () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'claude-digest-step-'));
    roots.push(root);
    const log = path.join(root, 'worker.log');
    const write = fs.promises.writeFile.bind(fs.promises);
    const failure = new Error('fixture write failed after a prefix');
    vi.spyOn(fs.promises, 'writeFile').mockImplementationOnce(async (file, _data, options) => {
      await write(file, '{', options);
      throw failure;
    });
    await expect(publishDigestStep(log, { event: 'failed-stop' })).rejects.toBe(failure);
    expect(fs.readdirSync(root)).toEqual([]);
  });
});
