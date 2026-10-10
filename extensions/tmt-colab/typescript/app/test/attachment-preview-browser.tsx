/** Read-lifecycle double only. Admitted reads have separate service/real-door tests. */
import '@tmt/browser-ui/static.css';
import '../src/style.css';
import { createRoot, type Root } from 'react-dom/client';
import { MessageAttachments } from '../src/message-attachments.js';
import type { CommentView } from '../src/thread-records.js';
import type { AttachmentBinding } from '../src/attachment-service.js';
import { AttachmentReadError } from '../src/attachments.js';
import { id } from './ask-fixtures.js';
let root: Root | undefined;
let scope = 'head:1',
  count = 7,
  revision = '1',
  kind = 'image/png';
let reads: string[] = [],
  cleared: Uint8Array[] = [];
let active = 0,
  maximum = 0,
  held = false;
let release: (() => void) | undefined;
let refused = false;
let activeBinding: AttachmentBinding;
const raw = Uint8Array.from(
  atob(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==',
  ),
  (c) => c.charCodeAt(0),
);
const binding: AttachmentBinding = {
  scope: () => scope,
  async limits() {
    return { payloadBytes: 1024 };
  },
  async upload() {
    throw new Error('Read-only fixture');
  },
  async resume() {
    throw new Error('Read-only fixture');
  },
  async discard() {
    throw new Error('Read-only fixture');
  },
  async open(reference, descriptor) {
    reads.push(
      `${descriptor.filename}:${reference.kind === 'message' ? reference.revision : 'document'}`,
    );
    active++;
    maximum = Math.max(maximum, active);
    try {
      if (held)
        await new Promise<void>((resolve) => {
          release = resolve;
        });
      if (refused) throw new AttachmentReadError('denied');
      const bytes = raw.slice();
      cleared.push(bytes);
      return bytes;
    } finally {
      active--;
    }
  },
};
function render() {
  const attachments = Array.from({ length: count }, (_, index) => ({
    attachmentId: id(index + 10),
    epoch: id(1),
    filename: `image-${index + 1}.png`,
    mediaType: kind,
    plaintextBytes: String(raw.length),
    source: {
      kind: 'message' as const,
      writerId: id(2),
      messageId: id(3),
      messageRevision: revision,
    },
  }));
  const comment = {
    ref: { writer: id(2), id: id(3) },
    messageId: id(3),
    revision,
    attachments,
  } as CommentView;
  root!.render(
    <div style={{ width: 'min(360px, calc(100vw - 28px))', margin: '14px' }}>
      <p className="comment-body">These images and the grid share this left edge.</p>
      <MessageAttachments comment={comment} binding={activeBinding} />
      <button type="button" style={{ marginTop: 1000 }}>
        After attachments
      </button>
    </div>,
  );
}
export function mount(
  options: { count?: number; held?: boolean; kind?: string; refused?: boolean } = {},
) {
  root?.unmount();
  document.getElementById('attachment-fixture')?.remove();
  const host = document.createElement('main');
  host.id = 'attachment-fixture';
  document.getElementById('root')?.setAttribute('hidden', '');
  document.body.append(host);
  root = createRoot(host);
  scope = 'head:1';
  revision = '1';
  reads = [];
  cleared = [];
  active = 0;
  maximum = 0;
  count = options.count ?? 7;
  held = options.held ?? false;
  kind = options.kind ?? 'image/png';
  refused = options.refused ?? false;
  activeBinding = binding;
  render();
}
export function advance() {
  scope = 'head:2';
  render();
}
export function revise() {
  revision = '2';
  render();
}
export function finish() {
  held = false;
  release?.();
}
export function replaceBinding() {
  activeBinding = { ...binding };
  render();
}
export function unmount() {
  root?.unmount();
}
export function proof() {
  return { reads, maximum, zeroed: cleared.every((bytes) => bytes.every((b) => b === 0)) };
}
