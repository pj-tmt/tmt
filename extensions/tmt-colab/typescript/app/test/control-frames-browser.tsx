/** Presentation-only controls: local text state, no transport or product effects. */
import 'virtual:tokens.css';
import '@tmt/browser-ui/static.css';
import '../src/style.css';
import { createRoot } from 'react-dom/client';
import { useState } from 'react';
import { Plus } from 'lucide-react';
import {
  BrowserAction,
  BrowserChips,
  BrowserCommand,
  BrowserField,
  BrowserIconAction,
  BrowserToggle,
} from '@tmt/browser-ui/react';
import { MessageComposer, type ComposerEdit } from '../src/components/message-composer.js';

function Controls() {
  const [edit, setEdit] = useState<ComposerEdit>({ value: 'A retained Colab draft' });
  const [query, setQuery] = useState('');
  return (
    <main
      id="control-frames"
      style={{ display: 'grid', gap: 20, maxWidth: 620, margin: 20, minHeight: 0 }}
    >
      <h1>Shared control frames</h1>
      <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap' }}>
        <BrowserAction type="button" variant="primary" label="Action" onActivate={() => {}} />
        <BrowserAction type="button" variant="secondary" label="Secondary" onActivate={() => {}} />
        <BrowserAction
          type="button"
          variant="primary"
          label="Disabled"
          disabled
          onActivate={() => {}}
        />
        <BrowserIconAction
          type="button"
          variant="primary"
          label="Icon action"
          icon={<Plus />}
          onActivate={() => {}}
        />
        <BrowserIconAction
          type="button"
          variant="primary"
          label="Disabled icon"
          icon={<Plus />}
          disabled
          onActivate={() => {}}
        />
      </div>
      <BrowserField
        controlId="frame-field"
        label="Shared field"
        renderControl={(props) => <textarea {...props} defaultValue="An editable draft" />}
      />
      <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap' }}>
        <BrowserToggle label="Toggle" pressed={false} onActivate={() => {}} />
        <BrowserToggle label="Checked toggle" pressed onActivate={() => {}} />
      </div>
      <BrowserChips
        label="Labels"
        items={[{ id: 'one', label: 'Design', removeLabel: 'Remove Design' }]}
        options={[{ id: 'two', label: 'Reviewed' }]}
        query={query}
        onQueryChange={setQuery}
        onAdd={() => {}}
        onRemove={() => {}}
        addLabel="Add label"
        searchLabel="Search labels"
        emptyText="No labels"
      />
      <BrowserCommand
        text="tmt status --json"
        action={
          <button type="button" className="tmt-ui-command-copy">
            Copy
          </button>
        }
      />
      <MessageComposer
        label="Colab composer"
        edit={edit}
        onChange={setEdit}
        disabled={false}
        accessory={<span>Pending attachment · draft.txt</span>}
      />
    </main>
  );
}

export function mountControlFrames() {
  createRoot(document.getElementById('root')!).render(<Controls />);
}
