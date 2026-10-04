import { beforeEach, expect, it, vi } from 'vite-plus/test';
import { TitleCache } from '../src/title-cache.js';
import { UPDATE_BYTES } from '../src/fold-protocol.js';
import { titleKey } from '../src/keyring.js';

const stored = new Map<string, unknown>();
let unavailable = false;
vi.mock('../src/storage.js', () => ({
  record: async (key: string, ...values: unknown[]) => {
    if (unavailable) throw new Error('Storage unavailable');
    if (values.length) stored.set(key, structuredClone(values[0]));
    else return structuredClone(stored.get(key));
  },
}));
beforeEach(() => {
  stored.clear();
  unavailable = false;
  vi.stubGlobal('navigator', {
    locks: { request: async (_name: string, action: () => unknown) => action() },
  });
});

it('encrypts title hints and restores them through persisted non-extractable key handles', async () => {
  const cache = new TitleCache('space', 'device');
  const title = 'Private title 🗝 <script>not markup</script>';
  await cache.remember('page', title);
  const record = structuredClone(stored.get('title:space:device:page'));
  expect(record).toEqual({ nonce: expect.any(String), ciphertext: expect.any(String) });
  expect(JSON.stringify(record)).not.toContain(title);
  const key = await titleKey('device');
  expect(key.extractable).toBe(false);
  await expect(crypto.subtle.exportKey('raw', key)).rejects.toThrow();
  expect(await new TitleCache('space', 'device').read('page')).toBe(title);
  await cache.remember('page', 'Renamed');
  expect(await new TitleCache('space', 'device').read('page')).toBe('Renamed');
  expect(stored.get('title:space:device:page')).not.toEqual(record);
});

it('binds ciphertext to space, device and page even if a stored record is copied', async () => {
  await new TitleCache('space', 'device').remember('page', 'Scoped title');
  const value = stored.get('title:space:device:page');
  // Hold the key constant so the device negative specifically proves AAD binding.
  stored.set('title-key:other', structuredClone(stored.get('title-key:device')));
  for (const [space, device, page] of [
    ['other', 'device', 'page'],
    ['space', 'other', 'page'],
    ['space', 'device', 'other'],
  ]) {
    stored.set(`title:${space}:${device}:${page}`, structuredClone(value));
    expect(await new TitleCache(space, device).read(page)).toBeUndefined();
  }
  expect(await new TitleCache('space', 'device').read('page')).toBe('Scoped title');
});

it('treats absent, malformed, tampered or unavailable hints as optional display data', async () => {
  const cache = new TitleCache('space', 'device');
  expect(await cache.read('page')).toBeUndefined();
  await cache.remember('page', 'Safe title');
  const good = stored.get('title:space:device:page') as { nonce: string; ciphertext: string };
  for (const value of [
    null,
    { ...good, extra: true },
    { ...good, nonce: 'AA' },
    { ...good, ciphertext: 'AAAA' },
  ]) {
    stored.set('title:space:device:page', value);
    expect(await cache.read('page')).toBeUndefined();
  }
  stored.set('title:space:device:page', good);
  unavailable = true;
  await expect(cache.remember('page', 'Lost hint')).resolves.toBeUndefined();
  expect(await cache.read('page')).toBeUndefined();
  unavailable = false;
  expect(await cache.read('page')).toBe('Safe title');
});

it('refuses oversized, aborted and unusable-key cache writes without replacing good hints', async () => {
  const cache = new TitleCache('space', 'device');
  await cache.remember('page', 'Current title');
  await cache.remember('page', 'x'.repeat(UPDATE_BYTES + 1));
  const abort = new AbortController();
  abort.abort();
  await cache.remember('page', 'Cancelled title', abort.signal);
  const during = new AbortController();
  const writing = cache.remember('page', 'Aborted during encryption', during.signal);
  during.abort();
  await writing;
  expect(await cache.read('page')).toBe('Current title');
  stored.set('title-key:device', { algorithm: { name: 'AES-GCM' } });
  await cache.remember('page', 'Wrong key');
  expect(await cache.read('page')).toBeUndefined();
  expect(stored.get('title-key:device')).toEqual({ algorithm: { name: 'AES-GCM' } });
});
