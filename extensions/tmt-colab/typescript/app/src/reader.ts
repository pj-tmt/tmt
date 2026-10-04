import {
  binary,
  certificate,
  encodeBinary,
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
import type { PageView } from './transport.js';

/** The link no longer grants access: reset, removed, narrowed or the page is gone. */
export class AccessEndedError extends Error {
  constructor() {
    super('Access ended');
  }
}
/** Page modes a link may read: `link`, or `public` where anyone may. */
const SHARING = ['link', 'public'];
const CERTIFICATE_SKEW_MS = 10 * 60 * 1000;
const CERTIFICATE_LIFETIME_MS = 24 * 60 * 60 * 1000;
const RETRIES = 5;

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
  #closed = false;
  #failures = 0;
  private constructor(
    readonly target: ReaderLink,
    readonly mount: URL,
    readonly link: linkKeys.LinkKeys,
    readonly device: { id: string; sign: CryptoKey; signPublic: Uint8Array<ArrayBuffer> },
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
    const link = await linkKeys.deriveLink(target.seed, target.space, target.link);
    target.seed.fill(0);
    const pair = (await crypto.subtle.generateKey('Ed25519', false, [
        'sign',
        'verify',
      ])) as CryptoKeyPair,
      signPublic = new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey)),
      device = { id: crypto.randomUUID(), sign: pair.privateKey, signPublic },
      now = Date.now();
    const chain = await linkKeys.certifyDevice(link, {
      deviceId: device.id,
      signingPublic: signPublic,
      encryptionPublic: link.encryption.publicKey(),
      membershipRevision: target.revision,
      issuerStatement: target.statement,
      issuedAt: now - CERTIFICATE_SKEW_MS,
      expiresAt: now + CERTIFICATE_LIFETIME_MS,
    });
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
        typeof granted.principal === 'string',
    );
    generatedId(granted.principal);
    const owner = binary(granted.ownerKey, 32, 32);
    const registration: Registration = {
      deviceId: this.device.id,
      keys: {
        sign: this.device.sign,
        signPublic: this.device.signPublic,
        enc: this.link.encryption,
      },
      chain,
    };
    const admission = new Admission(space, page, challenge.epoch, owner, registration, {
      linkId: this.target.link,
      principal: granted.principal as string,
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
          if (!this.#closed) this.publish(view);
        },
        resolve,
        ['colab-sync-v1', `colab-reader-v1.${encodeBinary(token)}`],
      );
    });
    token.fill(0);
    try {
      await connection.ready;
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
  close() {
    this.#closed = true;
    this.#connection?.close();
    this.#connection = null;
  }
}
