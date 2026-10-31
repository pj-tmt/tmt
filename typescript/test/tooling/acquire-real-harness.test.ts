import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vite-plus/test';
import e2eConfig from '../e2e/vitest.config.js';
import advisoryConfig from '../e2e/real-harness.config.js';
import {
  assertHarnessImage,
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
  it('inherits the ordinary E2E lifecycle setup while selecting only the advisory scenario', () => {
    expect(e2eConfig.test?.globalSetup).toEqual(['test/e2e/harness/lifecycle.ts']);
    expect(advisoryConfig.test?.globalSetup).toBe(e2eConfig.test?.globalSetup);
    expect(advisoryConfig.test?.pool).toBe(e2eConfig.test?.pool);
    expect(advisoryConfig.test?.include).toEqual(['test/e2e/real-harness-smoke.advisory.test.ts']);
    expect(advisoryConfig.test?.testTimeout).toBe(150_000);
    expect(advisoryConfig.test?.hookTimeout).toBe(30_000);
  });

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

describe('image-only acquisition admission', () => {
  it.each(['/.dockerenv', '/opt/tmt-real-harness-image-build'])(
    'accepts Linux image marker %s without fetching bytes',
    (marker) => {
      expect(() =>
        assertHarnessImage({ platform: 'linux', exists: (file) => file === marker })
      ).not.toThrow();
    }
  );

  it.each(['darwin', 'win32', 'linux'])(
    'refuses an unmarked %s host before acquisition',
    (platform) => {
      expect(() => assertHarnessImage({ platform, exists: () => false })).toThrow(
        'HARNESS_CONTAINER_REQUIRED'
      );
    }
  );

  it('does not treat a marker on a non-Linux host as an image', () => {
    expect(() => assertHarnessImage({ platform: 'darwin', exists: () => true })).toThrow(
      'HARNESS_CONTAINER_REQUIRED'
    );
  });
});
