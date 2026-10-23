import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vite-plus/test';
import {
  downloadHarnessArchive,
  validateHarnessPins,
} from '../../scripts/acquire-real-harness.mjs';

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});
function archivePath() {
  const root = mkdtempSync(path.join(tmpdir(), 'tmt-harness-archive-'));
  roots.push(root);
  return path.join(root, 'synthetic.tgz');
}
const bytes = Buffer.from('Synthetic archive bytes only; no provider payload.');
const item = {
  url: 'https://fixture.invalid/synthetic.tgz',
  integrity: `sha512-${createHash('sha512').update(bytes).digest('base64')}`,
};
const pins = () =>
  JSON.parse(readFileSync(new URL('../e2e/real-harness-pins.json', import.meta.url), 'utf8'));

describe('container harness acquisition boundary', () => {
  it('accepts the exact manifest versions and registry archive locations', () => {
    expect(validateHarnessPins(pins()).packages).toHaveLength(4);
  });

  it.each(['version', 'url', 'installDirectory', 'integrity'])(
    'refuses an invalid package %s before acquisition',
    (field) => {
      const manifest = pins();
      manifest.packages[0][field] = '../untrusted';
      expect(() => validateHarnessPins(manifest)).toThrow('HARNESS_PINS');
    }
  );

  it('refuses missing packages or a different version guard', () => {
    const missing = pins();
    missing.packages.pop();
    expect(() => validateHarnessPins(missing)).toThrow('HARNESS_PINS');
    const changed = pins();
    changed.versions.codex = 'codex-cli latest';
    expect(() => validateHarnessPins(changed)).toThrow('HARNESS_PINS');
  });

  it('streams verified synthetic bytes to the archive and disables redirects', async () => {
    const archive = archivePath();
    const fetchImpl: typeof fetch = async (url, options) => {
      expect(url).toBe(item.url);
      expect(options?.redirect).toBe('error');
      expect(options?.signal).toBeInstanceOf(AbortSignal);
      return new Response(bytes);
    };
    expect(await downloadHarnessArchive(item, archive, { fetchImpl })).toEqual({
      archiveBytes: bytes.length,
    });
    expect(readFileSync(archive)).toEqual(bytes);
  });

  it('removes an archive whose actual bytes differ from the pinned integrity', async () => {
    const archive = archivePath();
    await expect(
      downloadHarnessArchive(item, archive, {
        fetchImpl: async () => new Response(Buffer.from('Different synthetic bytes')),
      })
    ).rejects.toThrow('HARNESS_ARCHIVE_INTEGRITY');
    expect(existsSync(archive)).toBe(false);
  });

  it('bounds streamed bytes and removes the partial archive', async () => {
    const archive = archivePath();
    await expect(
      downloadHarnessArchive(item, archive, {
        maxBytes: bytes.length - 1,
        fetchImpl: async () => new Response(bytes),
      })
    ).rejects.toThrow('HARNESS_DOWNLOAD_LIMIT');
    expect(existsSync(archive)).toBe(false);
  });

  it('refuses a failed HTTP response without leaving an archive', async () => {
    const archive = archivePath();
    await expect(
      downloadHarnessArchive(item, archive, {
        fetchImpl: async () => new Response('Unavailable', { status: 503 }),
      })
    ).rejects.toThrow('HARNESS_DOWNLOAD: HTTP 503');
    expect(existsSync(archive)).toBe(false);
  });

  it('removes a partial archive if the response stream fails', async () => {
    const archive = archivePath();
    await expect(
      downloadHarnessArchive(item, archive, {
        fetchImpl: async () =>
          new Response(
            new ReadableStream({
              start(controller) {
                controller.enqueue(bytes.subarray(0, 8));
                controller.error(new Error('Synthetic transport failure'));
              },
            })
          ),
      })
    ).rejects.toThrow('Synthetic transport failure');
    expect(existsSync(archive)).toBe(false);
  });
});
