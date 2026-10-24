import { useState } from 'react';
import { Plus, LoaderCircle } from 'lucide-react';
import { BrowserAction, BrowserMenu, BrowserTabs } from '../src/react.js';
import type { BrowserActionVariant } from '../src/static.js';
import { NativeMenu } from './menu-host-frame.js';

/** Actual opt-in compact controls; this fixture owns only demonstration selection and busy state. */
export function CompactControlsFrame({ params }: { params: URLSearchParams }) {
  const presentation = params.get('presentation') === 'compact' ? 'compact' : undefined;
  const [selected, setSelected] = useState('all');
  const [sort, setSort] = useState('recent');
  const [busy, setBusy] = useState(false);
  const [activations, setActivations] = useState(0);
  const labels = ['All', 'Needs you', 'Discussion', 'In progress', 'Done', 'Archived'];
  const tabs = labels.map((label, index) => ({
    id: index === 0 ? 'all' : String(index),
    tabId: `compact-tab-${index}`,
    panelId: `compact-panel-${index}`,
    label,
    count: [6, 1, 3, 2, 1, 1][index],
  }));
  const items = [
    { key: 'recent', label: 'Recently updated', checked: sort === 'recent' },
    { key: 'title', label: 'Title', checked: sort === 'title' },
  ];
  const value = sort === 'recent' ? 'Recently updated' : 'Title';
  return (
    <main className="gallery-main gallery-stack" id="compact-controls">
      <div className="gallery-row">
        <h1>Page-list controls</h1>
        <BrowserAction
          type="button"
          variant="primary"
          label="New page"
          presentation={presentation}
          leadingIcon={<Plus />}
          busyMark={<LoaderCircle />}
          busy={busy}
          onActivate={() => setActivations((n) => n + 1)}
        />
      </div>
      <BrowserTabs
        label="Page states"
        tabs={tabs}
        selected={selected}
        variant="flat"
        wrap
        presentation={presentation}
        onSelect={setSelected}
      >
        {(tab) => (
          <label>
            {tab.label} draft
            <input aria-label={`${tab.label} draft`} defaultValue="Retained draft" />
          </label>
        )}
      </BrowserTabs>
      <div className="gallery-row">
        <BrowserMenu
          label={`Sort: ${value}`}
          triggerText={value}
          presentation={presentation}
          items={items}
          onSelect={setSort}
        />
        <NativeMenu
          options={{
            label: `Native sort: ${value}`,
            triggerText: value,
            presentation,
            items,
            onSelect: setSort,
          }}
        />
      </div>
      <p data-testid="compact-activations">{activations}</p>
      <button type="button" onClick={() => setBusy((value) => !value)}>
        Toggle fixture busy
      </button>
      <div className="gallery-row" data-testid="compact-variants">
        {(['primary', 'secondary', 'text', 'ghost', 'destructive'] as BrowserActionVariant[]).map(
          (variant) => (
            <BrowserAction
              key={variant}
              type="button"
              variant={variant}
              label={`${variant} action`}
              presentation={presentation}
              leadingIcon={<Plus />}
              busyMark={<LoaderCircle />}
              busy={busy}
              onActivate={() => setActivations((n) => n + 1)}
            />
          ),
        )}
      </div>
    </main>
  );
}
