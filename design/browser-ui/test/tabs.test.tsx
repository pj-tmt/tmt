import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import { BrowserTabs, type BrowserTab } from '../src/react.js';

const tabs: BrowserTab[] = [
  {
    id: 'one',
    tabId: 'tab-one',
    panelId: 'panel-one',
    label: 'Layout',
    state: { mark: '✓', word: 'Answered', tone: 'working' },
  },
  {
    id: 'two',
    tabId: 'tab-two',
    panelId: 'panel-two',
    label: 'Pages',
    state: { mark: '◆', word: 'Waiting', tone: 'waiting' },
  },
];
it('associates every persistent panel with its tab and keeps one selected tab stop', () => {
  const render = (selected: string) =>
    renderToStaticMarkup(
      <BrowserTabs
        label="Header decisions"
        tabs={tabs}
        selected={selected}
        onSelect={() => {}}
        status="1/2 answered"
      >
        {(tab) => <input aria-label={`${tab.label} draft`} defaultValue={tab.id} />}
      </BrowserTabs>,
    );
  for (const selected of ['one', 'two']) {
    const html = render(selected);
    expect(html).toContain('role="tablist" aria-label="Header decisions"');
    expect(html.match(/aria-selected="true"/g)).toHaveLength(1);
    expect(html.match(/tabindex="-1"/g)).toHaveLength(1);
    expect(html.match(/role="tabpanel"/g)).toHaveLength(2);
    expect(html.match(/hidden=""/g)).toHaveLength(1);
    expect(html.match(/tabindex="0"/g)).toHaveLength(1);
    const elements = html.match(/<(?:button|div)\b[^>]*>/g) ?? [];
    for (const tab of tabs) {
      const tabElement = elements.find((element) => element.includes(` id="${tab.tabId}"`));
      const panelElement = elements.find((element) => element.includes(` id="${tab.panelId}"`));
      expect(tabElement).toContain(`aria-controls="${tab.panelId}"`);
      expect(panelElement).toContain(`aria-labelledby="${tab.tabId}"`);
    }
    expect(html).toContain('aria-label="✓ Layout · Answered"');
    expect(html).toContain('aria-label="◆ Pages · Waiting"');
    expect(html).toContain('value="one"');
    expect(html).toContain('value="two"');
  }
});
it('lets the host opt panels into the Tab order', () => {
  const html = renderToStaticMarkup(
    <BrowserTabs label="Decisions" tabs={tabs} selected="one" onSelect={() => {}} panelTabIndex={0}>
      {(tab) => tab.label}
    </BrowserTabs>,
  );
  expect(html.match(/tabindex="0"/g)).toHaveLength(3);
});
