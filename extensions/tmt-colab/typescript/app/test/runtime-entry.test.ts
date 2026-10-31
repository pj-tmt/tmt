import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it, vi } from 'vite-plus/test';

it('exposes the curated icons on window.tmt so a page can write them as htm templates', async () => {
  vi.stubGlobal('window', globalThis);
  vi.stubGlobal('HTMLElement', class {});
  const registered = new Map<string, CustomElementConstructor>();
  vi.stubGlobal('customElements', {
    get: (name: string) => registered.get(name),
    define: (name: string, value: CustomElementConstructor) => registered.set(name, value),
  });
  await import('../src/runtime/entry.js');
  const { icons } = await import('../src/runtime/icons.js');
  const { browserElements } = await import('@tmt/browser-ui/elements-manifest');
  expect([...registered.keys()]).toEqual(browserElements.map((entry) => entry.tag));
  const tmt = (
    globalThis as unknown as {
      tmt: { html: (...args: unknown[]) => unknown; icons: typeof icons };
    }
  ).tmt;
  expect(tmt.icons).toBe(icons);
  expect(Object.isFrozen(tmt)).toBe(true);

  const { Check } = tmt.icons;
  const markup = renderToStaticMarkup(
    tmt.html`<button>${tmt.html`<${Check} size=${20} aria-label="Done" />`}</button>` as never,
  );
  expect(markup).toContain('<button><svg');
  expect(markup).toContain('width="20"');
  expect(markup).toContain('role="img"');
  expect(markup).toContain('stroke-linecap="square"');
});
