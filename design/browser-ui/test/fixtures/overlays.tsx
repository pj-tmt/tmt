import { useCallback, useState } from 'react';
import { BrowserDrawer, BrowserMenu } from '../../src/react.js';

export function Overlays() {
  const [open, setOpen] = useState(false);
  const [header, setHeader] = useState(0);
  const [selected, setSelected] = useState('first');
  const [reason, setReason] = useState('none');
  const [mounts, setMounts] = useState(0);
  const draftRef = useCallback((node: HTMLTextAreaElement | null) => {
    if (node) setMounts((value) => value + 1);
  }, []);
  return (
    <>
      <header className="tmt-ui-header">
        <button type="button" onClick={() => setOpen(true)}>
          Open drawer
        </button>
        <button type="button" onClick={() => setHeader(header + 1)}>
          Header action
        </button>
        <BrowserMenu
          label="Header menu"
          items={[
            { key: 'first', label: 'First' },
            { key: 'last', label: 'Last' },
          ]}
          onSelect={setSelected}
        />
      </header>
      <p>
        Header activations: <output data-testid="header-count">{header}</output>
      </p>
      <p>
        Selected: <output data-testid="selection">{selected}</output>
      </p>
      <p>
        Dismissal: <output data-testid="dismissal">{reason}</output>
      </p>
      <p>
        Draft mounts: <output data-testid="mounts">{mounts}</output>
      </p>
      <iframe
        title="Author content"
        srcDoc='<button onclick="document.body.dataset.presses = String(Number(document.body.dataset.presses || 0) + 1)" style="width:100%;height:300px">Author button</button>'
        style={{ width: '70%', height: 320 }}
      />
      <div style={{ position: 'fixed', bottom: 4, left: 4 }}>
        <BrowserMenu
          label="Bottom action"
          items={[
            { key: 'first', label: 'First' },
            { key: 'last', label: 'Last' },
          ]}
          onSelect={setSelected}
        />
      </div>
      <BrowserDrawer
        title="Fixture"
        caption="Drafts stay here"
        open={open}
        panel="fixture"
        onDismiss={(value) => {
          setReason(value);
          setOpen(false);
        }}
      >
        <textarea aria-label="Retained draft" defaultValue="Initial draft" ref={draftRef} />
        <BrowserMenu
          label="Choice"
          valueLabel={selected}
          items={[
            { key: 'first', label: 'First', checked: selected === 'first' },
            { key: 'last', label: 'Last', checked: selected === 'last' },
            {
              key: 'disabled',
              label: 'Unavailable',
              checked: false,
              disabled: true,
              disabledReason: 'Read only',
            },
          ]}
          onSelect={setSelected}
        />
      </BrowserDrawer>
    </>
  );
}
