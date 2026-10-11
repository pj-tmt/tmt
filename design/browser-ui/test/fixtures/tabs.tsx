import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserTabs, type BrowserTab } from '../../src/react.js';

const tabs: BrowserTab[] = Array.from({ length: 12 }, (_, index) => ({
  id: String(index + 1),
  tabId: `tab-${index + 1}`,
  panelId: `panel-${index + 1}`,
  label:
    index === 0
      ? 'Layout'
      : index === 1
        ? 'Pages'
        : index === 2
          ? 'W'.repeat(24)
          : `Decision ${index + 1}`,
  state:
    index === 0
      ? { mark: '✓', word: 'Answered', tone: 'working' }
      : { mark: '◆', word: 'Waiting', tone: 'waiting' },
}));
function Fixture() {
  const [selected, setSelected] = useState('2');
  return (
    <BrowserTabs
      label="Header decisions"
      tabs={tabs}
      selected={selected}
      status="1/12 answered"
      panelTabIndex={new URLSearchParams(location.search).has('focus-panel') ? 0 : undefined}
      onSelect={(id, reason, event) => {
        document.querySelector('#events')!.textContent = `${id}:${reason}:${event.isTrusted}`;
        setSelected(id);
      }}
    >
      {(tab) => (
        <label>
          {tab.label} draft <input aria-label={`${tab.label} draft`} defaultValue="" />
        </label>
      )}
    </BrowserTabs>
  );
}
createRoot(document.querySelector('#root')!).render(<Fixture />);
