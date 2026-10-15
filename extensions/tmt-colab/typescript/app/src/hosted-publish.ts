import { equal } from '@tmt/colab-client';
import type {
  SealedDocument,
  SealedSyncPort,
  SyncCall,
} from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';

export interface HostedPublishResult {
  outcome: 'sent' | 'refused' | 'unconfirmed' | 'writer-in-use';
  /** Complete originals proved by readback, never just commit acknowledgments. */
  confirmed: number;
  failure?: unknown;
}

/** Transport only. Colab prepares/seals own records and owns author admission before this call. */
export async function publishHostedOwn(
  port: Pick<SealedSyncPort, 'publish' | 'read'>,
  originals: readonly SealedDocument[],
  call: SyncCall,
): Promise<HostedPublishResult> {
  live(call);
  if (originals.length < 1 || originals.length > 4) throw { code: 'REMOTE_INPUT_INVALID' };
  const documents = originals.map((document) => ({
    ...document,
    target: { ...document.target },
    bytes: new Uint8Array(document.bytes),
  }));
  if (documents.some((document) => document.target.resource !== 'log'))
    throw { code: 'REMOTE_SCOPE_DENIED' };
  if (
    documents.some((document) => document.bytes.length > 393216) ||
    new Set(documents.map((document) => document.target.id)).size !== documents.length
  )
    throw { code: 'REMOTE_INPUT_INVALID' };
  let confirmed = 0;
  for (const document of documents) {
    let failure: unknown;
    try {
      live(call);
      const result = await port.publish({ mode: 'create', document }, call);
      // The SDK already performed its single exact-ID comparison for a create collision.
      if (result.kind === 'already-present') {
        confirmed++;
        continue;
      }
    } catch (error) {
      failure = error;
      if (conflict(error)) return { outcome: 'writer-in-use', confirmed, failure };
      if (beforeEffect(error))
        return { outcome: confirmed === 0 ? 'refused' : 'unconfirmed', confirmed, failure };
    }
    // Both an acknowledgment and an unknown need this one bounded exact-original observation.
    try {
      live(call);
      const actual = await port.read(document.target, call);
      live(call);
      if (
        actual &&
        actual.target.resource === document.target.resource &&
        actual.target.id === document.target.id
      ) {
        if (actual.digest === document.digest && equal(actual.bytes, document.bytes)) {
          confirmed++;
          continue;
        }
        return {
          outcome: 'writer-in-use',
          confirmed,
          failure: { code: 'REMOTE_STATE_UNAVAILABLE', state: 'conflict' },
        };
      }
    } catch (error) {
      failure = error;
    }
    return { outcome: 'unconfirmed', confirmed, ...(failure === undefined ? {} : { failure }) };
  }
  return { outcome: 'sent', confirmed };
}

function beforeEffect(value: unknown): boolean {
  if (typeof value !== 'object' || value === null || !('code' in value)) return false;
  return (
    [
      'REMOTE_SCOPE_DENIED',
      'REMOTE_BUDGET_EXHAUSTED',
      'REMOTE_RATE_LIMITED',
      'REMOTE_INPUT_INVALID',
    ].includes(String(value.code)) ||
    (value.code === 'REMOTE_STATE_UNAVAILABLE' &&
      'state' in value &&
      ['expired', 'sign-in-needed', 'cancelled', 'deadline', 'unsupported'].includes(
        String(value.state),
      ))
  );
}
function live(call: SyncCall): void {
  if (call.signal.aborted) throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' };
  if (!Number.isFinite(call.deadlineAtMs) || Date.now() >= call.deadlineAtMs)
    throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'deadline' };
}

function conflict(value: unknown): boolean {
  return (
    typeof value === 'object' &&
    value !== null &&
    'code' in value &&
    value.code === 'REMOTE_STATE_UNAVAILABLE' &&
    'state' in value &&
    value.state === 'conflict'
  );
}
