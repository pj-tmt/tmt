/** The shared-page reader of an editor link over a scripted save. Never imported by production. */
import { createRoot, type Root } from 'react-dom/client';
import { ReaderApp } from '../src/reader-app.js';
import { SaveConflict, SaveRefused } from '../src/save.js';

let root: Root | undefined;
const saves: { source: string; base: string }[] = [];
/** What the scripted serve answers to the next save. */
let answer: 'saved' | 'merged' | 'conflict' | 'full' = 'saved';

export function proof() {
  return { saves };
}
export function script(next: typeof answer) {
  answer = next;
}
export async function mount(role: 'viewer' | 'editor') {
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
  answer = 'saved';
  const editor = role === 'editor';
  root.render(
    <ReaderApp
      state={{
        kind: 'ready',
        view: { title: 'Release notes', source: '<main><h1>Release notes</h1></main>' },
      }}
      canEdit={editor}
      edit={
        editor
          ? async (source, base) => {
              saves.push({ source, base });
              if (answer === 'conflict') throw new SaveConflict(1, { line: 1, excerpt: 'Release' });
              if (answer === 'full')
                throw new SaveRefused('COLAB_CAPACITY', 'The page is full.', undefined);
              return {
                merged: answer === 'merged',
                source: answer === 'merged' ? `${source}!` : source,
              };
            }
          : undefined
      }
    />,
  );
}
