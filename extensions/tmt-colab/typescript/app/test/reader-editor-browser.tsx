/** The shared-page reader of an editor link over a scripted edit request. Never imported by
 * production. */
import { createRoot, type Root } from 'react-dom/client';
import { useRef, useState } from 'react';
import { BrowserMenu } from '@tmt/browser-ui/react';
import { ReaderApp } from '../src/reader-app.js';
import { OwnAppendRefused } from '../src/own-publication.js';
import { EditTooLarge } from '../src/save.js';

let root: Root | undefined;
const saves: { source: string; base: string }[] = [];
/** What the scripted page does after the next request: shows the text (`applied`), shows other
 * text (`changed`), stays as it was (`waiting`, until `settle`), or the request is `large`, or
 * another user of the link `raced` it to the stream's next sequence. */
let answer: 'applied' | 'changed' | 'waiting' | 'large' | 'raced' | 'refused' | 'unconfirmed' =
  'applied';
let release: ((source: string) => void) | undefined;

export function proof() {
  return { saves };
}
export function script(next: typeof answer) {
  answer = next;
}
/** The waiting request's page finally shows `source`. */
export function settle(source: string) {
  release?.(source);
}
export async function mount(role: 'viewer' | 'editor', metadata = false) {
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('reader-editor-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'reader-editor-fixture';
    document.body.append(host);
  }
  root?.unmount();
  root = createRoot(host);
  await import('../src/reader-style.css');
  saves.length = 0;
  answer = 'applied';
  release = undefined;
  const editor = role === 'editor';
  root.render(
    <ReaderApp
      state={{
        kind: 'ready',
        view: {
          title: 'Release notes',
          source: '<main><h1>Release notes</h1></main>',
          ...(metadata
            ? {
                originalAuthor: 'Original writer',
                creator: {
                  agent: { id: '10000000-0000-4000-8000-000000000001', name: 'Selected creator' },
                  attribution: { writerId: '10000000-0000-4000-8000-000000000002', timeMs: 1 },
                },
                questions: [
                  {
                    id: '10000000-0000-4000-8000-000000000003',
                    title: 'Long_question_title_'.repeat(7),
                    state: 'open' as const,
                    attribution: { writerId: '10000000-0000-4000-8000-000000000002', timeMs: 1 },
                  },
                  {
                    id: '10000000-0000-4000-8000-000000000004',
                    title: 'Handled reader question',
                    state: 'handled' as const,
                    attribution: { writerId: '10000000-0000-4000-8000-000000000002', timeMs: 1 },
                  },
                ],
                threads: [],
              }
            : {}),
        },
      }}
      canEdit={editor}
      edit={
        editor
          ? async (source, base, onWaiting) => {
              saves.push({ source, base });
              if (answer === 'refused' || answer === 'unconfirmed') return { request: answer };
              if (answer === 'large') throw new EditTooLarge(200 * 1024, 128 * 1024);
              if (answer === 'raced') throw new OwnAppendRefused('CONFLICT');
              if (answer === 'waiting') {
                onWaiting();
                return new Promise((resolve) => {
                  release = (shown) =>
                    resolve({ kind: shown === source ? 'applied' : 'changed', source: shown });
                });
              }
              return answer === 'changed'
                ? { kind: 'changed', source: `${source}!` }
                : { kind: 'applied', source };
            }
          : undefined
      }
    />,
  );
}

/** Exercise the shared trigger ref through an icon-to-field replacement. */
export function mountMenuReference() {
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('reader-editor-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'reader-editor-fixture';
    document.body.append(host);
  }
  root?.unmount();
  root = createRoot(host);
  function Probe() {
    const trigger = useRef<HTMLButtonElement>(null);
    const [field, setField] = useState(false);
    return (
      <>
        <BrowserMenu
          label="Ref menu"
          triggerRef={trigger}
          valueLabel={field ? 'Current' : undefined}
          items={[{ key: 'one', label: 'One' }]}
          onSelect={() => {}}
        />
        <button type="button" onClick={() => trigger.current?.focus()}>
          Focus trigger
        </button>
        <button type="button" onClick={() => setField(true)}>
          Replace trigger
        </button>
      </>
    );
  }
  root.render(<Probe />);
}
