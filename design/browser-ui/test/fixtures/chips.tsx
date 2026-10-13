import { StrictMode, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserChips, type BrowserChipItem, type BrowserChipOption } from '../../src/react.js';

const options: BrowserChipOption[] = [
  { id: 'ux', label: '@tmt-ux', description: 'UX' },
  { id: 'lead', label: '@colab-lead', description: 'Colab' },
  { id: 'one', label: '@colab-1', description: 'Colab' },
  { id: 'long', label: `@${'long-name-'.repeat(14)}`, description: 'Long label' },
  { id: 'disabled', label: '@unavailable', disabled: true },
  ...Array.from({ length: 8 }, (_, i) => ({ id: `agent-${i}`, label: `@agent-${i}` })),
];
const item = (option: BrowserChipOption): BrowserChipItem => ({
  id: option.id,
  label: option.label,
  removeLabel: `Remove ${option.label}`,
});
function Fixture() {
  const mode = new URLSearchParams(location.search).get('case');
  const [items, setItems] = useState<BrowserChipItem[]>(
    mode === 'cap'
      ? options.slice(5).map(item)
      : mode === 'empty'
        ? []
        : mode === 'long'
          ? [item(options[3]), item(options[0])]
          : [item(options[0])],
  );
  const [query, setQuery] = useState('');
  const [visible, setVisible] = useState(true);
  const [disabled, setDisabled] = useState(mode === 'disabled');
  const [reports, setReports] = useState<boolean[]>([]);
  const [events, setEvents] = useState<string[]>([]);
  const unavailable = mode === 'loading' || mode === 'failed';
  return (
    <>
      <label>
        Before recipients <input id="before" />
      </label>
      {visible && (
        <BrowserChips
          label="Recipients"
          items={items}
          options={
            unavailable
              ? []
              : options.filter((o) => o.label.toLowerCase().includes(query.toLowerCase()))
          }
          query={query}
          onQueryChange={setQuery}
          addLabel="Add recipient"
          searchLabel="Search agents"
          emptyText="Stays on the page, no one is asked"
          limitText="At most 8"
          maxItems={8}
          statusText={
            mode === 'loading'
              ? 'Loading agents'
              : mode === 'failed'
                ? 'Could not load agents'
                : undefined
          }
          disabled={disabled}
          addDisabled={unavailable}
          focusFallback={() => document.querySelector<HTMLInputElement>('#after')}
          onOpenChange={(open) => setReports((previous) => [...previous, open])}
          onAdd={(id, event) => {
            if (!event.isTrusted) return;
            setItems((previous) => [...previous, item(options.find((o) => o.id === id)!)]);
            setEvents((previous) => [...previous, `add:${id}:${event.isTrusted}`]);
          }}
          onRemove={(id, event) => {
            if (!event.isTrusted) return;
            setItems((previous) => previous.filter((i) => i.id !== id));
            setEvents((previous) => [...previous, `remove:${id}:${event.isTrusted}`]);
          }}
        />
      )}
      <label>
        After recipients <input id="after" />
      </label>
      <button id="unmount" onClick={() => setVisible(false)}>
        Unmount chips
      </button>
      <button id="disable" onClick={() => setDisabled(true)}>
        Disable chips
      </button>
      <output id="events">{events.join('|')}</output>
      <output id="visibility">{JSON.stringify(reports)}</output>
    </>
  );
}
createRoot(document.querySelector('#root')!).render(
  <StrictMode>
    <Fixture />
  </StrictMode>,
);
