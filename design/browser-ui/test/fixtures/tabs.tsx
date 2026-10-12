import { useRef, useState } from 'react';
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
  const events = useRef<string[]>([]);
  return (
    <BrowserTabs
      label="Header decisions"
      tabs={tabs}
      selected={selected}
      status="1/12 answered"
      panelTabIndex={new URLSearchParams(location.search).has('focus-panel') ? 0 : undefined}
      onSelect={(id, reason, event) => {
        const eventText = `${id}:${reason}:${event.isTrusted}`;
        events.current.push(eventText);
        const output = document.querySelector<HTMLElement>('#events')!;
        output.textContent = eventText;
        output.dataset.events = JSON.stringify(events.current);
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
