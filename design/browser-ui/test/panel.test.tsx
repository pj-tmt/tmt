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

it('keeps the default header and forwards host actions before Close without native prop leakage', () => {
  const base = { open: true, title: 'Discussion', mode: 'card' as const, onDismiss() {} };
  const omitted = renderToStaticMarkup(<BrowserPanel {...base}>Host draft</BrowserPanel>);
  const undefinedActions = renderToStaticMarkup(
    <BrowserPanel {...base} headerActions={undefined}>
      Host draft
    </BrowserPanel>,
  );
  expect(undefinedActions).toBe(omitted);
  expect(omitted).not.toContain('tmt-ui-drawer-header-actions');
  const html = renderToStaticMarkup(
    <BrowserPanel
      {...base}
      headerActions={
        <button type="button" aria-label="Resolve thread" disabled>
          Resolve
        </button>
      }
    >
      Host draft
    </BrowserPanel>,
  );
  const header = html.slice(html.indexOf('<header'), html.indexOf('</header>'));
  expect(header).toContain('class="tmt-ui-drawer-header-actions"');
  expect(header).toContain('aria-label="Resolve thread" disabled=""');
  expect(header.indexOf('Resolve thread')).toBeLessThan(header.indexOf('Close Discussion'));
  expect(html).not.toContain('headerActions=');
  expect(html).not.toMatch(/<dialog|aria-modal|inert=/);
});

it('allows a host to supply Close text while preserving the existing defaults', () => {
  const base = { open: true, title: 'Discussion', mode: 'card' as const, onDismiss() {} };
  const normal = renderToStaticMarkup(<BrowserPanel {...base}>Draft</BrowserPanel>);
  const explicitDefaults = renderToStaticMarkup(
    <BrowserPanel {...base} closeLabel={undefined} closeTooltipLabel={undefined}>
      Draft
    </BrowserPanel>,
  );
  expect(explicitDefaults).toBe(normal);
  expect(normal).toContain('aria-label="Close Discussion"');
  expect(normal).toContain('Close Discussion · Esc');
  const custom = renderToStaticMarkup(
    <BrowserPanel {...base} closeLabel="Close (Esc)" closeTooltipLabel="Close (Esc)">
      Draft
    </BrowserPanel>,
  );
  expect(custom).toContain('aria-label="Close (Esc)"');
  expect(custom).toContain('>Close (Esc)</span>');
  expect(custom).not.toContain('closeLabel=');
  expect(custom).not.toContain('closeTooltipLabel=');
});

it('renders a host overlay inside the panel after the body', () => {
  const html = renderToStaticMarkup(
    <BrowserPanel open title="Chat" mode="card" onDismiss={() => {}} overlay={<p>Drop here</p>}>
      Draft
    </BrowserPanel>,
  );
  expect(html.indexOf('Drop here')).toBeGreaterThan(html.indexOf('Draft'));
  expect(html.indexOf('Drop here')).toBeLessThan(html.indexOf('</aside>'));
  expect(html).not.toContain('overlay=');
});
