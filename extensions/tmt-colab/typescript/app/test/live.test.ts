import { expect, it, vi } from 'vite-plus/test';
import type { Bootstrap, PageInfo } from '../src/bootstrap.js';
import type { Registration } from '../src/registration.js';
import { Live } from '../src/live.js';

const connections = vi.hoisted(() => [] as { failed(error: Error): void }[]);
vi.mock('../src/admission.js', () => ({
  Admission: class {
    async restore() {}
  },
}));
vi.mock('../src/connection.js', () => ({
  Connection: class {
    ready = Promise.resolve({ source: 'verified', title: 'Page' });
    constructor(
      _admission: unknown,
      _mount: URL,
      _sharing: string,
      publish: (value: { source: string; title: string }) => void,
      readonly failed: (error: Error) => void,
    ) {
      connections.push(this);
      publish({ source: 'verified', title: 'Page' });
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
