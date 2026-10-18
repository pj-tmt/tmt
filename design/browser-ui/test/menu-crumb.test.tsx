import { describe, expect, it } from 'vite-plus/test';
import { renderToStaticMarkup } from 'react-dom/server';
import { readFileSync } from 'node:fs';
import { BrowserMenu } from '../src/menu.js';

const items = [{ key: 'all', label: 'All pages' }];

describe('menu crumb trigger', () => {
  it('shows only the value and the chevron, named by its label and value', () => {
    const html = renderToStaticMarkup(
      <BrowserMenu
        label="Project"
        valueLabel="All pages"
        variant="crumb"
        items={items}
        onSelect={() => {}}
      />,
    );
    expect(html).toContain('data-variant="crumb"');
    expect(html).toContain('aria-label="Project: All pages"');
    expect(html).toContain('tmt-ui-menu-value">All pages</span>');
    expect(html).not.toContain('tmt-ui-menu-field-label');
    expect(html).toContain('aria-haspopup="menu"');
  });
  it('keeps the field trigger as it was by default', () => {
    const html = renderToStaticMarkup(
      <BrowserMenu label="Sort" valueLabel="Title" items={items} onSelect={() => {}} />,
    );
    expect(html).not.toContain('data-variant');
    expect(html).toContain('tmt-ui-menu-field-label">Sort</span>');
  });
  it('truncates the value and never the chevron', () => {
    const css = readFileSync(new URL('../generated/static.css', import.meta.url), 'utf8');
    expect(css).toContain(".tmt-ui-menu-trigger[data-variant='crumb'] > .tmt-ui-menu-value {");
    expect(css).toMatch(/\.tmt-ui-menu-value \{[^}]*text-overflow: ellipsis/);
    expect(css).toContain('.tmt-ui-menu-trigger > svg {\n  flex: none;');
  });
});

describe('menu item separator', () => {
  it('draws a rule above a separated row only', () => {
    const css = readFileSync(new URL('../generated/static.css', import.meta.url), 'utf8');
    expect(css).toContain('.tmt-ui-menu-item[data-separated] {\n  border-top: 1px solid');
  });
});
