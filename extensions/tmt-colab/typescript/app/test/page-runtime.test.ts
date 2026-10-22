import { afterEach, expect, test, vi } from 'vite-plus/test';
import { loadRuntime, runtimeUrl } from '../src/page-runtime.js';

afterEach(() => vi.unstubAllGlobals());
test('the asset sits next to the renderer: mount-relative, or the root on the hosted origin', () => {
  expect(runtimeUrl('http://127.0.0.1:4179/r/abcdefghijklmnop/x/colab/')).toBe(
    'http://127.0.0.1:4179/r/abcdefghijklmnop/x/colab/assets/colab-runtime.js',
  );
  expect(runtimeUrl('https://tmt-colab.web.app/s/anything')).toBe(
    'https://tmt-colab.web.app/assets/colab-runtime.js',
  );
});
test('the runtime is fetched once per path and a failure is not remembered', async () => {
  const fetched: string[] = [];
  let ok = false;
  vi.stubGlobal('fetch', async (url: string) => {
    fetched.push(url);
    return ok ? new Response('window.tmt = 1;') : new Response('no', { status: 500 });
  });
  const base = 'http://127.0.0.1:4179/r/aaaaaaaaaaaaaaaa/x/colab/';
  await expect(loadRuntime(base)).rejects.toThrow('Page runtime unavailable');
  ok = true;
  const first = await loadRuntime(base);
  expect(first).toEqual({ script: 'window.tmt = 1;', style: '' });
  expect(await loadRuntime(base)).toBe(first);
  expect(fetched).toHaveLength(2);
});
