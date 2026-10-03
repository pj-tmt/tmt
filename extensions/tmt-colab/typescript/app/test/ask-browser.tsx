/** Browser-test entry only; never imported by production routing or builds. */
import { createRoot, type Root } from 'react-dom/client';
import { binary, strictVerify } from '@tmt/colab-client';
import type { StoredAskDraft } from '../src/ask-record-store.js';
import { AskPreview } from '../src/ask-preview.js';
import { record } from '../src/storage.js';
import { destination, id, RemoteDouble, selection } from './ask-fixtures.js';
import { fixtureAttempt } from './ask-browser-attempt.js';
let root: Root | undefined;
let active: Awaited<ReturnType<typeof fixtureAttempt>> & { issuedAt: number; edit(): void };
export async function mount(
  options: {
    hold?: boolean;
    unavailable?: boolean;
    mode?: RemoteDouble['mode'];
    operationId?: string;
    issuedAt?: number;
  } = {},
) {
  root?.unmount();
  document.getElementById('ask-fixture')?.remove();
  document.getElementById('root')?.setAttribute('hidden', '');
  const host = document.createElement('div');
  host.id = 'ask-fixture';
  document.body.append(host);
  const input = selection(),
    target = destination();
  target.mode = options.hold ? 'hold' : null;
  const issuedAt = options.issuedAt ?? Date.now();
  const fixture = await fixtureAttempt(input, target, { ...options, issuedAt });
  active = {
    ...fixture,
    issuedAt,
    edit() {
      input.quote = 'changed live source';
      target.agent = id(99);
    },
  };
  root = createRoot(host);
  root.render(<AskPreview attempt={fixture.attempt} close={() => root?.unmount()} />);
}
export function editLive() {
  active.edit();
}
export async function proof() {
  const { attempt, remote, issuedAt, keys } = active,
    preview = attempt.preview;
  const draft = await record<StoredAskDraft>(`ask:${id(4)}:${preview.view.operationId}`);
  return {
    sends: remote.sends,
    reads: remote.reads,
    state: attempt.state.state,
    message: preview.view.message,
    deliveredMessage: preview.view.deliveredMessage,
    operationId: preview.view.operationId,
    issuedAt,
    draft,
    verified:
      !!draft &&
      (await strictVerify(
        keys.signPublic,
        binary(draft.signature, 64, 64),
        binary(draft.input, 16384),
      )),
    keyExportDenied: await crypto.subtle.exportKey('pkcs8', keys.sign).then(
      () => false,
      () => true,
    ),
  };
}
