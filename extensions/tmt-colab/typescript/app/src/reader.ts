import { AttachmentService, disclosureScope } from './attachment-service.js';
import {
  binary,
  certificate,
  encodeBinary,
  equal,
  exactKeys,
  generatedId,
  link as linkKeys,
  requireValue,
  sign,
} from '@tmt/colab-client';
import { Admission } from './admission.js';
import { Connection } from './connection.js';
import { jsonResponse, type Registration } from './registration.js';
import type { ReaderLink } from './reader-link.js';
import { deriveReaderKeys } from './reader-keys.js';
import { PageAnswer, sendEditRequest, type EditOutcome } from './link-edit.js';
import type { PageView } from './transport.js';
import { memoryRecords, Writer } from './writer.js';

/** The link no longer grants access: reset, removed, narrowed or the page is gone. */
export class AccessEndedError extends Error {
  constructor() {
    super('Access ended');
  }
}
/** Page modes a link may read: `link`, or `public` where anyone may. */
const SHARING = ['link', 'public'];
const RETRIES = 5;
/** How long an edit waits for the session to reconnect before it is not sent. */
const EDIT_CONNECT_MS = 10_000;

async function post(mount: URL, path: string, body: unknown) {
  const response = await fetch(new URL(path, mount), {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(10_000),
  });
  if (response.status === 403 || response.status === 400) throw new AccessEndedError();
  return jsonResponse(response, 8192);
}
export const accessEnded = (error: Error) =>
  error instanceof AccessEndedError || ['DENIED', 'STALE_EPOCH'].includes(error.message);

/** One link device. The seed and the keys derived from it stay in memory, in this tab only. */
export class ReaderSession {
  #connection: Connection | null = null;
  /** Read-only: authorized reads of the page's files over the current connection. */
  readonly attachments = new AttachmentService({
    connection: async () => {
      if (!this.#connection) throw new Error('Reader unavailable');
      return this.#connection;
    },
    available: () => !this.#closed && this.#connection !== null,
    sharing: SHARING,
    scope: () => disclosureScope(this.#connection),
  });
  #closed = false;
  #failures = 0;
  /** The source of the latest page the connection admitted, which a sent edit waits to see change. */
  readonly #page = new PageAnswer();
  /** Sends this link device's edit requests; created by its first edit. */
  #writer: Writer | null = null;
  private constructor(
    readonly target: ReaderLink,
    readonly mount: URL,
    readonly link: linkKeys.LinkKeys,
    readonly device: linkKeys.DerivedDevice,
    readonly chain: Uint8Array,
    readonly publish: (view: PageView) => void,
    readonly ended: (error: Error) => void,
  ) {}
  /** Consumes the parsed link: the seed is wiped before any request is made. Rejects when the
   * first connection cannot be established; later drops reconnect until access ends. */
  static async open(
    target: ReaderLink,
    mount: URL,
    publish: (view: PageView) => void,
    ended: (error: Error) => void,
  ): Promise<ReaderSession> {
    const { link, device, chain } = await deriveReaderKeys(target, 'door');
    const session = new ReaderSession(target, mount, link, device, chain, publish, ended);
    const { dropped } = await session.#establish();
    void session.#supervise(dropped);
    return session;
  }
  /** Challenge, signature, ticket, then one sync tunnel carrying the ticket as a subprotocol.
   * Resolves once page catchup completes; `dropped` settles when that tunnel drops. */
  async #establish(): Promise<{ dropped: Promise<Error> }> {
    const { space, page } = this.target;
    const chain = certificate.Chain.fromJson(this.chain);
    const challenge = await post(this.mount, 'api/readers/challenge', {
      kind: 'link',
      space,
      page,
      chain: encodeBinary(this.chain),
    });
    exactKeys(challenge, [
      'challengeId',
      'nonce',
      'space',
      'page',
      'epoch',
      'chainDigest',
      'expiresAt',
    ]);
    requireValue(
      challenge.space === space &&
        challenge.page === page &&
        typeof challenge.challengeId === 'string' &&
        typeof challenge.epoch === 'string' &&
        typeof challenge.expiresAt === 'number',
    );
    const digest = binary(challenge.chainDigest, 32, 32);
    requireValue(encodeBinary(digest) === encodeBinary(await chain.digest()));
    const input = linkKeys.readerSessionInput({
      challengeId: challenge.challengeId,
      nonce: binary(challenge.nonce, 32, 32),
      space,
      page,
      epoch: challenge.epoch,
      chainDigest: digest,
      expiresAt: challenge.expiresAt,
    });
    const granted = await post(this.mount, 'api/readers/session', {
      kind: 'link',
      challengeId: challenge.challengeId,
      signature: encodeBinary(await sign(this.device.sign, input)),
    });
    exactKeys(granted, ['principal', 'token', 'space', 'page', 'epoch', 'ownerKey', 'expiresAt']);
    requireValue(
      granted.space === space &&
        granted.page === page &&
        granted.epoch === challenge.epoch &&
        typeof granted.principal === 'string' &&
        typeof granted.expiresAt === 'number' &&
        Number.isSafeInteger(granted.expiresAt) &&
        granted.expiresAt > Date.now(),
    );
    generatedId(granted.principal);
    const owner = binary(granted.ownerKey, 32, 32);
    // The link names its owner: a door that presents another key is not this link's space.
    if (!equal(owner, this.target.root)) throw new AccessEndedError();
    const registration: Registration = {
      deviceId: this.device.id,
      keys: {
        sign: this.device.sign,
        signPublic: this.device.signingPublic,
        enc: this.link.encryption,
      },
      chain,
    };
    const admission = new Admission(space, page, challenge.epoch, owner, registration, {
      linkId: this.target.link,
      principal: granted.principal as string,
      expiresAt: granted.expiresAt,
    });
    // The link names the space; the owner key the server presents must derive that exact ID.
    await admission.restore();
    const token = binary(granted.token, 32, 32);
    let connection!: Connection;
    const dropped = new Promise<Error>((resolve) => {
      connection = new Connection(
        admission,
        this.mount,
        SHARING,
        (view) => {
          this.#page.show(view.source);
          if (!this.#closed) this.publish(view);
        },
        resolve,
        ['colab-sync-v1', `colab-reader-v1.${encodeBinary(token)}`],
      );
    });
    token.fill(0);
    try {
      await connection.ready;
      if (!this.#closed) connection.send('subscribe', { cursors: [] });
    } catch (error) {
      connection.close();
      throw error;
    }
    if (this.#closed) connection.close();
    else this.#connection = connection;
    this.#failures = 0;
    return { dropped };
  }
  async #supervise(dropped: Promise<Error>) {
    let error = await dropped;
    for (;;) {
      this.#connection = null;
      if (this.#closed) return;
      if (accessEnded(error)) return this.ended(new AccessEndedError());
      if (++this.#failures > RETRIES) return this.ended(error);
      await new Promise((resolve) =>
        setTimeout(resolve, Math.min(500 * 2 ** this.#failures, 15_000)),
      );
      try {
        if (this.#closed) return;
        error = await (await this.#establish()).dropped;
      } catch (next) {
        error = next instanceof Error ? next : new Error('Reader unavailable');
      }
    }
  }
  /** Ask the page's owner to save page text: an edit request, sealed into this link device's own
   * stream, which the owner's Colab applies or does not. A request that is over what one carries
   * is refused before anything is sent. Once it is sent, the page is the only answer: this
   * resolves when it shows the proposed text (`applied`) or anything else (`changed`), and calls
   * `onWaiting` once if the page has not changed after `EDIT_WAITING_MS`. Never sent again. */
  async edit(source: string, base: string, onWaiting: () => void = () => {}): Promise<EditOutcome> {
    if (this.#closed) throw new Error('Page editing unavailable');
    const before = this.#page.shown;
    // A link session stores nothing in the owner app's records: its writer keeps its state in
    // memory, for the life of the tab.
    this.#writer ??= new Writer(
      `colab-link-writer:${this.target.space}:${this.device.id}`,
      () => this.#active(),
      memoryRecords(),
    );
    await sendEditRequest(this.#writer, base, source);
    return this.#page.answer(source, before, onWaiting);
  }
  /** The connection to write over, waiting a short while for a dropped one to return. */
  async #active(): Promise<Connection> {
    const deadline = Date.now() + EDIT_CONNECT_MS;
    for (;;) {
      if (this.#closed) throw new Error('Page editing unavailable');
      const connection = this.#connection;
      if (connection?.active) return connection;
      if (Date.now() >= deadline) throw new Error('Reader unavailable');
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }
  close() {
    this.#closed = true;
    this.#writer?.close();
    this.#writer = null;
    this.#connection?.close();
    this.#connection = null;
    this.#page.end();
  }
}
