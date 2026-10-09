/** Baseline overlay geometry. Fixture controls never prepare or dispatch a real Ask. */
import 'virtual:tokens.css';
import '@tmt/browser-ui/static.css';
import '../src/colab-header.css';
import '../src/style.css';
import { createRoot } from 'react-dom/client';
import { useState } from 'react';
import { MessageComposer, type ComposerEdit } from '../src/components/message-composer.js';
import { mount } from './annotation-browser.js';

if (!new URL(location.href).searchParams.has('field')) {
  mount('multi');
  const popover = document.getElementById('annotation-fixture')!;
  popover.className = 'annotation-popover';
  popover.setAttribute('role', 'dialog');
  popover.setAttribute('aria-label', 'Annotation baseline');
  popover.style.left = '12px';
  popover.style.top = '80px';
  // The reused fixture host is <main>; production SelectionAnnotation uses <div>.
  popover.style.minHeight = '0';
  const toolbar = document.createElement('nav');
  toolbar.setAttribute('aria-label', 'Fixture position');
  for (const position of ['Top', 'Bottom'] as const) {
    const button = document.createElement('button');
    button.textContent = position;
    button.addEventListener('click', () => {
      popover.style.top = position === 'Top' ? '80px' : 'auto';
      popover.style.bottom = position === 'Bottom' ? '12px' : 'auto';
    });
    toolbar.append(button);
  }
  document.body.prepend(toolbar);
}

/** Access changes retain one editor root and its session history. */
export function mountField() {
  const host = document.getElementById('root')!;
  function FieldFixture() {
    const [edit, setEdit] = useState<ComposerEdit>({ value: '' });
    const [disabled, setDisabled] = useState(false);
    return (
      <>
        <button type="button" onClick={() => setDisabled(!disabled)}>
          Toggle access
        </button>
        <MessageComposer
          label="Retained message"
          edit={edit}
          onChange={setEdit}
          disabled={disabled}
        />
      </>
    );
  }
  createRoot(host).render(<FieldFixture />);
}
