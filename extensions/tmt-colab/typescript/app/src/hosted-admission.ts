import {
  binary,
  digest,
  equal,
  exactKeys,
  generatedId,
  spaceId,
  strictJson,
} from '@tmt/colab-client';
import type {
  SealedDocument,
  SealedSyncPort,
  SyncCall,
} from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';

/** Derived by Colab's link model; the role remains a Rules-checked hint, never authority. */
export interface HostedLinkAdmission {
  space: string;
  page: string;
  link: string;
  role: 'viewer' | 'commenter' | 'editor';
  token: string;
}
/** Routing/lifetime hints only. Signed membership/epoch/wraps still decide what can be read. */
export interface HostedPageHints {
  epoch: string;
  expiresAtMs: number;
}
export class HostedAdmission {
  private attempt?: Promise<void>;
  private identity?: string;
  async open(
    port: SealedSyncPort,
    link: HostedLinkAdmission,
    call: SyncCall,
  ): Promise<HostedPageHints> {
    live(call);
    const input = { ...link };
    const { space, page, token, role } = input;
    try {
      spaceId(space);
      generatedId(page);
      generatedId(input.link);
      binary(token, 32, 32);
      if (!['viewer', 'commenter', 'editor'].includes(role)) throw invalid();
    } catch {
      throw invalid();
    }
    if (space !== port.context.space) throw denied();
    const uid = port.context.uid;
    const bytes = new TextEncoder().encode(
      JSON.stringify({
        link: { stringValue: input.link },
        page: { stringValue: page },
        role: { stringValue: role },
        seq: { integerValue: '1' },
        space: { stringValue: space },
        token: { stringValue: token },
        uid: { stringValue: uid },
      }),
    );
    const desired: SealedDocument = {
      target: { resource: 'members', id: `${space}_${page}_${uid}` },
      bytes,
      digest: Array.from(await digest(bytes), (byte) => byte.toString(16).padStart(2, '0')).join(
        '',
      ),
    };
    live(call);
    // A reader owns one parsed link. Keep the failed attempt as well, so recovery never retries a create.
    if (this.identity !== undefined && this.identity !== desired.digest) throw denied();
    this.identity = desired.digest;
    this.attempt ??= this.admitOnce(port, desired, call);
    await this.attempt;
    live(call);
    const target = { resource: 'pages', id: `${space}_${page}` };
    const document = await port.read(target, call);
    live(call);
    if (
      !document ||
      document.target.resource !== target.resource ||
      document.target.id !== target.id
    )
      throw unavailable();
    try {
      const fields = strictJson(document.bytes, 4096);
      exactKeys(fields, ['space', 'page', 'epoch', 'state', 'expiresAt']);
      if (
        string(fields.space) !== space ||
        string(fields.page) !== page ||
        !['open', 'archived'].includes(string(fields.state))
      )
        throw unavailable();
      exactKeys(fields.epoch, ['integerValue']);
      const epoch = fields.epoch.integerValue;
      if (
        typeof epoch !== 'string' ||
        !/^[1-9][0-9]*$/.test(epoch) ||
        !Number.isSafeInteger(Number(epoch))
      )
        throw unavailable();
      exactKeys(fields.expiresAt, ['timestampValue']);
      const timestamp = fields.expiresAt.timestampValue;
      if (
        typeof timestamp !== 'string' ||
        !/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{3}(?:\d{3})?)?Z$/.test(timestamp)
      )
        throw unavailable();
      const expiresAtMs = Date.parse(timestamp);
      if (
        !Number.isSafeInteger(expiresAtMs) ||
        expiresAtMs <= Date.now() ||
        new Date(expiresAtMs).toISOString().slice(0, 19) !== timestamp.slice(0, 19)
      )
        throw unavailable();
      return { epoch, expiresAtMs };
    } catch {
      throw unavailable();
    }
  }
  private async admitOnce(
    port: SealedSyncPort,
    desired: SealedDocument,
    call: SyncCall,
  ): Promise<void> {
    let existing: SealedDocument | undefined;
    try {
      existing = await port.read(desired.target, call);
    } catch (error) {
      live(call);
      // Rules deny a get of a missing own member. Only this exact bootstrap target can
      // proceed to one create; the create's Rules answer is final, with no role probing.
      if (
        typeof error !== 'object' ||
        error === null ||
        !('code' in error) ||
        error.code !== 'REMOTE_SCOPE_DENIED'
      )
        throw error;
    }
    live(call);
    if (existing) {
      matching(existing, desired);
      return;
    }
    const outcome = await port.publish({ mode: 'create', document: desired }, call);
    live(call);
    if (outcome.kind === 'unknown') {
      const readback = await port.read(desired.target, call);
      live(call);
      if (!readback) throw unavailable();
      matching(readback, desired);
    } else if (outcome.kind !== 'confirmed' && outcome.kind !== 'already-present')
      throw unavailable();
  }
}

function matching(actual: SealedDocument, desired: SealedDocument): void {
  if (
    actual.target.resource !== desired.target.resource ||
    actual.target.id !== desired.target.id ||
    !equal(actual.bytes, desired.bytes)
  )
    throw denied();
}
function string(value: unknown): string {
  exactKeys(value, ['stringValue']);
  if (typeof value.stringValue !== 'string') throw unavailable();
  return value.stringValue;
}
function live(call: SyncCall): void {
  if (call.signal.aborted) throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' };
  if (!Number.isFinite(call.deadlineAtMs) || Date.now() >= call.deadlineAtMs)
    throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'deadline' };
}
const denied = () => ({ code: 'REMOTE_SCOPE_DENIED' as const });
const invalid = () => ({ code: 'REMOTE_INPUT_INVALID' as const });
const unavailable = () => ({ code: 'REMOTE_STATE_UNAVAILABLE' as const });
