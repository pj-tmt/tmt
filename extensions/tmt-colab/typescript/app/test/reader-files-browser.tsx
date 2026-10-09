/** Read-only reader with a files list over a deterministic read double. Never imported by production. */
import { createRoot, type Root } from 'react-dom/client';
import type { attachment } from '@tmt/colab-client';
import type { AttachmentBinding } from '../src/attachment-service.js';
import { ReaderApp } from '../src/reader-app.js';

let root: Root | undefined;
let opened: string[] = [];
const PNG = Uint8Array.from(
  atob(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==',
  ),
  (c) => c.charCodeAt(0),
);
const descriptor = (name: string, mediaType: string, size: number) =>
  ({
    attachmentId: crypto.randomUUID(),
    filename: name,
    mediaType,
    plaintextBytes: String(size),
  }) as unknown as attachment.AttachmentDescriptor;

export function opens() {
  return opened;
}
export async function mount(files: boolean) {
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('reader-files-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'reader-files-fixture';
    document.body.append(host);
  }
  root?.unmount();
  root = createRoot(host);
  await import('../src/reader-style.css');
  opened = [];
  const list = files
    ? [
        descriptor('report.txt', 'application/octet-stream', 12),
        descriptor('shot.png', 'image/png', PNG.length),
        descriptor('gone.bin', 'application/octet-stream', 5),
      ]
    : [];
  const binding: Pick<AttachmentBinding, 'open'> = {
    async open(reference, d) {
      opened.push(`${reference.kind}:${d.filename}`);
      if (d.filename === 'gone.bin') throw new Error('Unavailable');
      return d.filename === 'shot.png' ? PNG.slice() : new TextEncoder().encode('read-only txt');
    },
  };
  root.render(
    <ReaderApp
      state={{
        kind: 'ready',
        view: {
          title: 'Release notes',
          source: '<main><h1>Release notes</h1></main>',
          ...(files ? { attachments: list } : {}),
        },
      }}
      attachments={binding as AttachmentBinding}
    />,
  );
}
