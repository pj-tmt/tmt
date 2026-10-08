/** Editing-only fixture: no Writer, Ask, storage or transport capabilities. */
import { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { MessageComposer } from '../src/components/message-composer.js';
import type { ComposerEdit } from '../src/components/message-composer-edit.js';
import { destination, id } from './ask-fixtures.js';
import 'virtual:tokens.css';
import '@tmt/browser-ui/static.css';
import '../src/colab-header.css';
import '../src/style.css';

let snapshot: ComposerEdit = { value: '' };
let submitted = 0;
function Fixture() {
  const [edit, setEdit] = useState<ComposerEdit>({ value: '' });
  const [resetKey, reset] = useState(0);
  const [shown, show] = useState(true);
  useEffect(() => {
    snapshot = edit;
  }, [edit]);
  return (
    <section>
      <button onClick={() => setEdit({ ...edit })}>Echo</button>
      <button
        onClick={() => {
          setEdit({ value: 'Reset value' });
          reset((key) => key + 1);
        }}
      >
        Reset
      </button>
      <button onClick={() => show(false)}>Close</button>
      <button onClick={() => show(true)}>Reopen</button>
      {shown && (
        <MessageComposer
          label="Editing fixture"
          edit={edit}
          onChange={setEdit}
          resetKey={resetKey}
          disabled={false}
          autoFocus
          candidates={[destination(), { ...destination(), agent: id(9), agentName: 'Other agent' }]}
          onSubmit={(event) => {
            if (event.isTrusted && !event.isComposing && event.keyCode !== 229) submitted++;
          }}
        />
      )}
    </section>
  );
}
createRoot(document.getElementById('root')!).render(<Fixture />);
export function proof() {
  return { edit: snapshot, submitted };
}
