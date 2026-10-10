/** Editing-only fixture: no transport, recipient directory or publish/dispatch port. */
import 'virtual:tokens.css';
import '@tmt/browser-ui/static.css';
import '../src/style.css';
import { createRoot } from 'react-dom/client';
import { useState } from 'react';
import {
  RichInput,
  plainRichInputDocument,
  type RichInputEdit,
} from '../src/components/rich-input.js';
import { MessageComposer, type ComposerEdit } from '../src/components/message-composer.js';

function Fixture() {
  const [edit, setEdit] = useState<RichInputEdit>({ value: '' });
  const [message, setMessage] = useState<ComposerEdit>({ value: '' });
  const [disabled, setDisabled] = useState(false);
  const [accessory, setAccessory] = useState(true);
  const [reset, setReset] = useState(0);
  const [applied, setApplied] = useState('');
  return (
    <main style={{ minHeight: 0, maxWidth: 500, margin: 16 }}>
      <RichInput
        label="Shared text"
        edit={edit}
        onChange={setEdit}
        document={plainRichInputDocument}
        disabled={disabled}
        resetKey={reset}
        accessory={accessory ? <span>Fixture accessory</span> : undefined}
        onKeyDownCapture={(event) => {
          // Fixture-only trusted toggle while the input keeps its focus.
          if (event.altKey && event.key === 'a') {
            event.preventDefault();
            setAccessory((value) => !value);
          }
          if (event.key === 'Enter' && !event.shiftKey) {
            event.preventDefault();
            setApplied(edit.value);
          }
          if (event.key === 'Escape') {
            event.preventDefault();
            setEdit({ value: '' });
            setReset((value) => value + 1);
          }
        }}
      />
      <output aria-label="Applied text">{applied}</output>
      <button type="button" onClick={() => setDisabled(!disabled)}>
        Toggle access
      </button>
      <MessageComposer
        label="Message with accessory"
        edit={message}
        onChange={setMessage}
        disabled={false}
        accessory={<span>Message accessory</span>}
      />
    </main>
  );
}
createRoot(document.getElementById('root')!).render(<Fixture />);
