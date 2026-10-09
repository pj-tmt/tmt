import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import { BrowserList, BrowserListRow } from '../src/react.js';

it('renders a named semantic list and host-owned navigation, metadata, state and actions', () => {
  const html = renderToStaticMarkup(
    <BrowserList label="Pages">
      <BrowserListRow
        data-page-id="active"
        title={<a href="/active">Active page</a>}
        metadata={<time dateTime="2026-10-09T00:00:00Z">Just now</time>}
        state="Private"
        actions={
          <details>
            <summary>Actions</summary>Host actions
          </details>
        }
      />
      <BrowserListRow
        data-page-id="archived"
        title="Archived page"
        metadata="Update time unknown"
        state="Archived"
      />
    </BrowserList>,
  );
  expect(html).toContain('<ul class="tmt-ui-list" aria-label="Pages">');
  expect(html.match(/<li/g)).toHaveLength(2);
  expect(html.match(/<a /g)).toHaveLength(1);
  expect(html.indexOf('href="/active"')).toBeLessThan(html.indexOf('<summary>'));
  expect(html).toContain('data-page-id="archived"');
  expect(html).toContain('tmt-ui-list-state">Archived');
  expect(html.match(/<time /g)).toHaveLength(1);
});

it('owns symmetric toggle inset and row rhythm independently of the old host padding token', () => {
  const css = readFileSync(new URL('../generated/static.css', import.meta.url), 'utf8');
  expect(css).not.toContain('--tmt-ui-host-toggle-padding');
  expect(css).toContain('padding: var(--tmt-ui-toggle-gap);');
  expect(css).toContain('--tmt-ui-list-padding-block: 10px;');
  expect(css).toContain('--tmt-ui-list-padding-inline: 14px;');
  expect(css).toContain(
    'padding: var(--tmt-ui-list-padding-block) var(--tmt-ui-list-padding-inline);',
  );
  expect(css).toContain("grid-template-areas: 'title actions' 'meta actions';");
  expect(css).not.toContain('__compact-max-width__');
});
