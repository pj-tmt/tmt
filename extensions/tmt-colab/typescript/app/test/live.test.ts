import { expect, it, vi } from 'vite-plus/test';
import type { Bootstrap, PageInfo } from '../src/bootstrap.js';
import type { Registration } from '../src/registration.js';
import { Live } from '../src/live.js';

const connections = vi.hoisted(
  () =>
    [] as {
      failed(error: Error): void;
      admission: {
        head: { revision: bigint; hash: Uint8Array } | null;
        root: object | null;
        validatePage: ReturnType<typeof vi.fn>;
      };
    }[],
);
vi.mock('../src/admission.js', () => ({
  Admission: class {
    head = { revision: 2n, hash: new Uint8Array(32).fill(10) };
    root = {};
    validatePage = vi.fn();
    constructor(
      readonly space: string,
      readonly page: string,
      readonly epoch: string,
    ) {}
    async restore() {}
  },
}));
vi.mock('../src/connection.js', () => ({
  Connection: class {
    ready = Promise.resolve({ source: 'verified', title: 'Page' });
    constructor(
      readonly admission: (typeof connections)[number]['admission'],
      _mount: URL,
      _sharing: string,
      publish: (value: { source: string; title: string }) => void,
      readonly failed: (error: Error) => void,
    ) {
      connections.push(this);
      publish({ source: 'verified', title: 'Page' });
    }
    async run<T>(fn: () => Promise<T>) {
      return fn();
    }
    close() {}
  },
}));
vi.mock('../src/writer.js', () => ({
  Writer: class {
    close() {}
  },
}));

it('successful catchup resets reconnect failures across the page lifetime', async () => {
  connections.length = 0;
  const live = new Live(
    new URL('https://example.test/colab/'),
    { space: 'space', owner: new Uint8Array(32) } as Bootstrap,
    { deviceId: 'device' } as Registration,
    { pageId: 'page', epoch: '1', sharing: 'private' } as PageInfo,
  );
  const failed = vi.fn();
  live.subscribe(() => {}, failed);
  try {
    await live.snapshot();
    for (let attempt = 0; attempt < 5; attempt++) {
      connections.at(-1)!.failed(new Error('Sync disconnected'));
      expect((await live.snapshot()).source).toBe('verified');
      expect(connections).toHaveLength(attempt + 2);
    }
    expect(failed).not.toHaveBeenCalled();
  } finally {
    live.close();
  }
});

it('exports admitted committed view/head and denies blocked, missing-key and closed bindings', async () => {
  const live = new Live(
    new URL('https://example.test/colab/'),
    { space: 'uqvpga22vglwpngpg7jd7zpk5vzjtoud', owner: new Uint8Array(32) } as Bootstrap,
    { deviceId: 'device' } as Registration,
    { pageId: '00000000-0000-4000-8000-000000000002', epoch: '1', sharing: 'private' } as PageInfo,
  );
  try {
    await live.snapshot();
    const c = connections.at(-1)!;
    const bundle = await live.export();
    expect(await bundle.blob('page.html').text()).toBe('verified');
    const manifest = JSON.parse(await bundle.blob('manifest.json').text());
    expect(manifest.title).toBe('Page');
    expect(manifest.membershipHead).toEqual({ revision: '2', statementHash: '0a'.repeat(32) });
    expect(manifest.epoch).toBe('1');
    expect(c.admission.validatePage).toHaveBeenCalledWith('private');
    c.admission.root = null;
    await expect(live.export()).rejects.toThrow();
    c.admission.root = {};
    c.admission.head = null;
    await expect(live.export()).rejects.toThrow();
    c.failed(new Error('Invalid signature'));
    await expect(live.export()).rejects.toThrow();
  } finally {
    live.close();
  }
  await expect(live.export()).rejects.toThrow();
});
