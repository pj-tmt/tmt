import {
  copy,
  coreId,
  decimal,
  decodeText,
  digest,
  encodeBinary,
  frame,
  generatedId,
  idList,
  requireValue,
  sign,
  spaceId,
  text,
  time,
  type Bytes,
} from '@tmt/colab-client';

export const REQUEST_BYTES = 1024 * 1024;
const HOUR = 60 * 60 * 1000;

/** Trusted parent input after page/role, source/render and member admission.
 * A renderer message or a claimed member name is never this admission. */
export interface AdmittedSelection {
  space: string;
  page: string;
  thread: string;
  messageIds: readonly string[];
  senderDevice: string;
  quote: string;
  comment: string;
  title: string;
  url: string;
}
/** Caller-verified machine/grant snapshot; this primitive creates no authority.
 * Revision and session expiry reference the verified Remote grant; unavailable policy stays null. */
export interface AskDestination {
  machine: string;
  machineName: string;
  online: 'online' | 'offline' | 'unknown';
  agent: string;
  agentName: string;
  grantExpiresAt: number | null;
  deviceName: string;
  grantRevision: string;
  mode: 'direct' | 'hold' | null;
}
export interface SignedAsk {
  readonly operationId: string;
  readonly senderDevice: string;
  readonly input: string;
  readonly signature: string;
  readonly finalBytes: string;
}

/** No capability crosses into the renderer. Strings are immutable; byte getters
 * return copies. Signing never rereads live source or a mutable selection. */
export class FrozenAsk {
  readonly view: Readonly<
    AskDestination & { message: string; deliveredMessage: string; operationId: string }
  >;
  readonly expiresAt: number;
  #scope: Readonly<AdmittedSelection>;
  #ids: Bytes;
  #final: Bytes;
  #issuedAt: number;
  private constructor(
    selection: AdmittedSelection,
    destination: AskDestination,
    operationId: string,
    issuedAt: number,
    validityMs: number,
    inputLimit: number,
  ) {
    spaceId(selection.space);
    for (const id of [
      selection.page,
      selection.thread,
      selection.senderDevice,
      destination.machine,
      operationId,
    ])
      generatedId(id);
    coreId(destination.agent);
    decimal(destination.grantRevision);
    time(issuedAt);
    requireValue(Number.isSafeInteger(validityMs) && validityMs > 0 && validityMs <= 24 * HOUR);
    this.expiresAt = issuedAt + validityMs;
    time(this.expiresAt);
    requireValue(destination.mode === null || ['direct', 'hold'].includes(destination.mode));
    if (destination.grantExpiresAt !== null) time(destination.grantExpiresAt);
    requireValue(['online', 'offline', 'unknown'].includes(destination.online));
    for (const value of [
      selection.quote,
      selection.comment,
      selection.title,
      selection.url,
      destination.agentName,
      destination.machineName,
      destination.deviceName,
    ])
      text(value);
    const url = new URL(selection.url);
    requireValue(['http:', 'https:'].includes(url.protocol) && !url.username && !url.password);
    url.hash = '';
    const message = `Page: ${selection.title}\nLink: ${url.href}\n\nQuote:\n${selection.quote}\n\nComment:\n${selection.comment}`;
    requireValue(Number.isSafeInteger(inputLimit) && inputLimit > 0 && inputLimit <= REQUEST_BYTES);
    this.#final = text(message);
    requireValue(this.#final.length <= inputLimit);
    this.#ids = idList(selection.messageIds);
    this.#scope = Object.freeze({
      ...selection,
      url: url.href,
      messageIds: Object.freeze([...selection.messageIds]),
    });
    this.#issuedAt = issuedAt;
    const deliveredMessage = `[remote: ${destination.deviceName}]\n${message}`;
    requireValue(text(deliveredMessage).length <= inputLimit);
    this.view = Object.freeze({ ...destination, operationId, message, deliveredMessage });
    Object.freeze(this);
  }
  static capture(
    selection: AdmittedSelection,
    destination: AskDestination,
    options: {
      operationId?: string;
      issuedAt?: number;
      validityMs?: number;
      inputLimit?: number;
    } = {},
  ) {
    return new FrozenAsk(
      selection,
      destination,
      options.operationId ?? crypto.randomUUID(),
      options.issuedAt ?? Date.now(),
      options.validityMs ?? HOUR,
      options.inputLimit ?? REQUEST_BYTES,
    );
  }
  deliveredBytes(): Bytes {
    return text(this.view.deliveredMessage);
  }
  finalBytes(): Bytes {
    return copy(this.#final);
  }
  async signed(key: CryptoKey, now = Date.now()): Promise<SignedAsk> {
    time(now);
    requireValue(now >= this.#issuedAt && now < this.expiresAt);
    const s = this.#scope,
      d = this.view;
    const input = frame(
      text('tmt-colab-send-v1'),
      text('1'),
      text(s.space),
      text(s.page),
      text(s.thread),
      this.#ids,
      text(d.machine),
      text(d.agent),
      text(d.operationId),
      await digest(this.#final),
      text(s.senderDevice),
      text(d.grantRevision),
      text(d.grantExpiresAt === null ? 'none' : String(d.grantExpiresAt)),
      text(String(this.#issuedAt)),
      text(String(this.expiresAt)),
    );
    return Object.freeze({
      operationId: d.operationId,
      senderDevice: s.senderDevice,
      input: encodeBinary(input),
      signature: encodeBinary(await sign(key, input)),
      finalBytes: encodeBinary(this.#final),
    });
  }
}

/** Separate escaped view exposes controls and Unicode formatting characters;
 * it never replaces the exact signed UTF-8 message. */
export function escapedPreview(bytes: Uint8Array): string {
  return decodeText(bytes).replace(
    /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/gu,
    (c) => `\\u{${c.codePointAt(0)!.toString(16)}}`,
  );
}
