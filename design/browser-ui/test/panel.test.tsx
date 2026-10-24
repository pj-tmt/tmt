import { expect, it } from 'vite-plus/test';
import { renderToStaticMarkup } from 'react-dom/server';
import { BrowserPanel } from '../src/panel.js';

it('labels a non-modal aside with its heading and shares the drawer header/caption', () => {
  const html = renderToStaticMarkup(
    <BrowserPanel open title="Discussion" caption="1 / 2" mode="card" onDismiss={() => {}}>
      Host draft
    </BrowserPanel>,
  );
  const label = html.match(/aria-labelledby="([^"]+)"/)![1];
  expect(html).toContain('<aside');
  expect(html).toContain(`id="${label}" tabindex="-1"`);
  expect(html).toContain('tmt-ui-drawer-header');
  expect(html).toContain('1 / 2');
  expect(html).toContain('Host draft');
  expect(html).not.toMatch(/<dialog|aria-modal|inert=/);
});

it('does not keep a closed panel or host draft focusable', () => {
  expect(
    renderToStaticMarkup(
      <BrowserPanel open={false} title="Discussion" mode="column" onDismiss={() => {}}>
        Host draft
      </BrowserPanel>,
    ),
  ).toBe('');
});
