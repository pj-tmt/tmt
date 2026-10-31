import { staticCss } from './static-css.js';
import { describe, expect, it } from 'vite-plus/test';
import { renderToStaticMarkup } from 'react-dom/server';
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
    const css = staticCss;
    expect(css).toContain(".tmt-ui-menu-trigger[data-variant='crumb'] > .tmt-ui-menu-value {");
    expect(css).toMatch(/\.tmt-ui-menu-value \{[^}]*text-overflow: ellipsis/);
    expect(css).toContain('.tmt-ui-menu-trigger > svg {\n  flex: none;');
  });
});

describe('menu item separator', () => {
  it('shares the rule for separated rows and group headings', () => {
    const css = staticCss;
    const rule = css.match(
      /:is\(\.tmt-ui-menu-item, \.tmt-ui-menu-heading\)\[data-separated\]::before \{([^}]+)\}/,
    )?.[1];
    expect(rule).toContain('border-top: var(--tmt-ui-rule-width) solid var(--tmt-ui-surface-rule)');
    expect(rule).toContain('border-radius: 0');
  });
});

describe('menu text trigger', () => {
  it('shows the text and the chevron, named by its label, without a field label', () => {
    const html = renderToStaticMarkup(
      <BrowserMenu
        label="Add to project"
        triggerText="Add to project"
        items={items}
        onSelect={() => {}}
        openRequest={3}
      />,
    );
    expect(html).toContain('aria-label="Add to project"');
    expect(html).toContain('tmt-ui-menu-value">Add to project</span>');
    expect(html).not.toContain('tmt-ui-menu-field-label');
    expect(html).not.toContain('tmt-ui-icon-action');
    // A request a menu mounts with opens nothing.
    expect(html).not.toContain('<dialog');
  });
});
