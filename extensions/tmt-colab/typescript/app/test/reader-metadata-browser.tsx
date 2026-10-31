import { MetadataDialogs } from '../src/metadata-dialogs.js';
import type { PageBinding } from '../src/transport.js';
/** Read-only UI/provenance double. Real owner admission is covered by acceptance/reader.spec.ts. */
import { createRoot, type Root } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { canonical, digest, encodeBinary, metadataRequest, text } from '@tmt/colab-client';
import { ReaderApp } from '../src/reader-app.js';
import { MetadataRequests } from '../src/link-metadata.js';
import type { JsonValue } from '../src/fold-protocol.js';
import type { PageView } from '../src/transport.js';
import '@tmt/browser-ui/static.css';
import '../src/reader-style.css';
const id = (n: number) => `10000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
let root: Root | undefined;
let view: PageView;
let role: 'editor' | 'viewer' | 'commenter';
let controller: MetadataRequests;
let component = ReaderApp;
let referenceShown = false;
let generation = 0;
let mode: 'applied' | 'refused' | 'waiting' | 'unconfirmed' = 'applied';
const requests: metadataRequest.MetadataRequest[] = [];
const saves: { source: string; base: string }[] = [];
let pending: { request: metadataRequest.MetadataRequest; controller: MetadataRequests } | undefined;
function paint() {
  if (referenceShown) {
    const unavailable = async (): Promise<never> => {
      throw new Error('Capture-only reference never writes');
    };
    const binding: PageBinding = {
      subscribe: () => () => {},
      edit: unavailable,
      export: unavailable,
      close: () => {},
      canMetadata: () => true,
      metadataRevision: async () => `v1:${'a'.repeat(64)}`,
      metadata: unavailable,
    };
    root!.render(
      <MetadataDialogs kind="questions" view={view} binding={binding} onDismiss={() => {}} />,
    );
    return;
  }
  const Component = component;
  root!.render(
    <Component
      state={{ kind: 'ready', pageId: id(generation + 100), view }}
      canEdit={role === 'editor'}
      metadata={role === 'editor' ? controller : undefined}
      edit={
        role === 'editor'
          ? async (source, base) => {
              saves.push({ source, base });
              view = { ...view, source };
              paint();
              return { kind: 'applied', source };
            }
          : undefined
      }
    />,
  );
}
async function answer(
  request: metadataRequest.MetadataRequest,
  recipient: MetadataRequests,
  refused = false,
) {
  if (recipient === controller && !refused) {
    view =
      request.action === 'creator.set'
        ? {
            ...view,
            creator: {
              agent: { id: id(10), name: request.body.selector },
              attribution: { writerId: id(2), timeMs: 2 },
            },
          }
        : {
            ...view,
            questions: view.questions?.map((entry) =>
              entry.id === request.body.id
                ? { ...entry, state: request.action === 'question.handle' ? 'handled' : 'open' }
                : entry,
            ),
          };
    paint();
  }
  const value = metadataRequest.editResult({
    version: 1,
    kind: 'edit-result',
    writer: id(1),
    operationId: request.operationId,
    requestSha256: encodeBinary(await digest(text(canonical(request)))),
    result: refused
      ? {
          operationId: request.operationId,
          state: 'rejected',
          code: 'COLAB_IDENTITY_UNKNOWN',
          message: 'No saved agent matches that name or ID.',
        }
      : { operationId: request.operationId, state: 'committed', revision: `v1:${'a'.repeat(64)}` },
  });
  recipient.show(
    {
      [id(2)]: {
        threads: {},
        intents: {},
        replies: {},
        messages: {
          [metadataRequest.outcomeKey(id(1), request.operationId)]: value as unknown as JsonValue,
        },
      },
    },
    (writer) => writer === id(2),
  );
}
export async function mount(nextRole: typeof role = 'editor') {
  controller?.end();
  root?.unmount();
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('reader-metadata-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'reader-metadata-fixture';
    document.body.append(host);
  }
  root = createRoot(host);
  role = nextRole;
  component = ReaderApp;
  referenceShown = false;
  generation++;
  mode = 'applied';
  requests.length = 0;
  saves.length = 0;
  view = {
    title: 'Editor metadata',
    source: `<h1>Editor metadata</h1><tmt-questions data-id="${id(20)}"><p data-title>Questions</p><tmt-question data-id="${id(3)}" data-kind="short"><p data-question>First question</p></tmt-question></tmt-questions>`,
    creator: {
      agent: { id: id(9), name: 'Current creator' },
      attribution: { writerId: id(2), timeMs: 1 },
    },
    questions: [
      {
        id: id(3),
        title: 'First question',
        state: 'open',
        attribution: { writerId: id(2), timeMs: 1 },
      },
      {
        id: id(4),
        title: 'Handled question removed from source',
        state: 'handled',
        attribution: { writerId: id(2), timeMs: 1 },
      },
    ],
    threads: [],
  };
  const active = new MetadataRequests(`fixture:${generation}`, id(1), async (request) => {
    requests.push(request);
    if (mode === 'waiting' || mode === 'unconfirmed') {
      pending = { request, controller: active };
      return mode === 'waiting' ? 'sent' : 'unconfirmed';
    }
    await answer(request, active, mode === 'refused');
    return 'sent';
  });
  controller = active;
  paint();
}
export function script(next: typeof mode) {
  mode = next;
}
export async function settle() {
  if (pending) await answer(pending.request, pending.controller);
}
export function archived() {
  view = { ...view, archived: true };
  paint();
}
export function legacy() {
  view = { ...view, questions: undefined };
  paint();
}
export function proof() {
  return { requests, saves, view };
}
/** Optional capture-only frozen production component, recorded before implementation. */
export function reference() {
  referenceShown = true;
  paint();
}
export async function before(modulePath: string) {
  referenceShown = false;
  const module = (await import(/* @vite-ignore */ modulePath)) as { ReaderApp: typeof ReaderApp };
  component = module.ReaderApp;
  paint();
}
export function after() {
  referenceShown = false;
  component = ReaderApp;
  paint();
}

/** Change only verified layout metadata; retain the same reader component and source. */
export function layout(next: PageView['layout']) {
  flushSync(() => {
    view = { ...view, layout: next };
    paint();
  });
}

/** Replace only authored source through the existing controlled reader view. */
export function source(next: string) {
  view = { ...view, source: next };
  paint();
}
