/** Browser-test entry only; never imported by production routing or builds. */
import { createRoot, type Root } from 'react-dom/client';
import { binary, strictVerify } from '@tmt/colab-client';
import { AskAttempt } from '../src/ask-attempt.js';
import { FrozenAsk } from '../src/ask-intent.js';
import { AskPreview } from '../src/ask-preview.js';
import type { Delivery } from '../src/ask-remote.js';
import { deviceKeys } from '../src/keyring.js';
import { record } from '../src/storage.js';
import { destination, id, RemoteDouble, selection } from './ask-fixtures.js';
let root: Root | undefined;
let active: { attempt: AskAttempt; remote: RemoteDouble; issuedAt: number; edit(): void };
export async function mount(
  options: {
    delivery?: Delivery;
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
    target = destination(),
    remote = new RemoteDouble();
  target.delivery = options.delivery;
  target.mode = options.hold ? 'hold' : 'direct';
  remote.mode = options.mode ?? 'accepted';
  const issuedAt = options.issuedAt ?? Date.now();
  const frozen = FrozenAsk.capture(input, target, { issuedAt, operationId: options.operationId });
  const keys = await deviceKeys(id(4));
  const attempt = new AskAttempt(
    frozen,
    keys.sign,
    undefined,
    options.unavailable ? undefined : remote,
  );
  active = {
    attempt,
    remote,
    issuedAt,
    edit() {
      input.quote = 'changed live source';
      target.agent = id(99);
    },
  };
  root = createRoot(host);
  root.render(
    <AskPreview
      attempt={attempt}
      close={() => {
        root?.unmount();
      }}
    />,
  );
}
export function editLive() {
  active.edit();
}
export async function proof() {
  const { attempt, remote, issuedAt } = active,
    preview = attempt.preview;
  const draft = await record<{ input: string; signature: string; finalBytes: string }>(
    `ask:${id(4)}:${preview.view.operationId}`,
  );
  const keys = await deviceKeys(id(4));
  return {
    sends: remote.sends,
    reads: remote.reads,
    state: attempt.state.state,
    message: preview.view.message,
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
