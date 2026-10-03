import {
  binary,
  certificate,
  decimal,
  equal,
  encodeBinary,
  exactKeys,
  requireValue,
  statement,
  payload,
  wrap,
} from '@tmt/colab-client';
import { record } from './storage.js';
import { verifyRegistration, type Registration } from './registration.js';

/** Owner-browser content subset. A transport head/hash is never log authority. */
export class Admission {
  head: statement.Head | null = null;
  #log: statement.Verified[] = [];
  #raw: string[] = [];
  #target: { revision: bigint; hash: Uint8Array } | null = null;
  root: CryptoKey | null = null;
  #authors = new Map<string, certificate.Certificate>();
  constructor(
    readonly space: string,
    readonly page: string,
    readonly epoch: string,
    readonly owner: Uint8Array,
    readonly registration: Registration,
  ) {}
  async restore() {
    await verifyRegistration(this.registration, this.space, this.owner);
    for (const raw of (await record<string[]>(`log:${this.space}`)) ?? []) await this.#next(raw);
    this.#authors.set(this.registration.deviceId, this.registration.chain.certificate());
  }
  async #next(raw: string) {
    const envelope = statement.Envelope.fromJson(binary(raw, 1024 * 1024)),
      verified = await envelope.verifyNext(this.space, this.owner, this.head);
    this.head = verified.head;
    this.#log.push(verified);
    this.#raw.push(encodeBinary(envelope.toJson()));
  }
  async membership(value: unknown, first: boolean) {
    exactKeys(
      value,
      first
        ? ['revision', 'statementHash', 'ownerKey', 'statements', 'more']
        : ['statements', 'more'],
    );
    if (first) {
      requireValue(
        equal(binary(value.ownerKey, 32, 32), this.owner) && typeof value.revision === 'string',
      );
      const revision = decimal(value.revision),
        hash = binary(value.statementHash, 32, 32);
      requireValue(!this.head || revision >= this.head.revision);
      if (this.head?.revision === revision) requireValue(equal(hash, this.head.hash));
      this.#target = { revision, hash };
    }
    requireValue(
      this.#target !== null &&
        Array.isArray(value.statements) &&
        value.statements.length <= 64 &&
        typeof value.more === 'boolean',
    );
    for (const raw of value.statements) {
      requireValue(typeof raw === 'string');
      await this.#next(raw);
    }
    requireValue(this.head !== null && this.head.revision <= this.#target.revision);
    if (!value.more)
      requireValue(
        this.head.revision === this.#target.revision && equal(this.head.hash, this.#target.hash),
      );
    requireValue(
      this.#raw.length <= 4096 &&
        this.#raw.reduce((n, raw) => n + raw.length, 0) <= 4 * 1024 * 1024,
    );
    await navigator.locks.request(`colab-log:${this.space}`, async () => {
      const previous = await record<string[]>(`log:${this.space}`);
      // Another tab may have advanced. Never replace its higher/forked durable head.
      if (previous && previous.length > this.#raw.length) {
        requireValue(this.#raw.every((v, i) => previous[i] === v));
      } else {
        requireValue(!previous || previous.every((v, i) => this.#raw[i] === v));
        await record(`log:${this.space}`, this.#raw);
      }
    });
  }
  async chains(values: unknown) {
    requireValue(Array.isArray(values) && values.length <= 64 && this.head !== null);
    const verified: certificate.Certificate[] = [];
    for (const value of values) {
      exactKeys(value, ['deviceId', 'chain']);
      const chain = certificate.Chain.fromJson(binary(value.chain, 16 * 1024)),
        c = chain.certificate();
      const issuer = this.#log[Number(decimal(c.membershipRevision)) - 1];
      requireValue(
        c.deviceId === value.deviceId &&
          c.space === this.space &&
          c.issuerKind === 'member' &&
          c.issuerId === this.head.ownerMember.id &&
          issuer !== undefined &&
          c.issuedAt <= Date.now() &&
          c.expiresAt > Date.now() &&
          !this.#log.some(
            (v) =>
              v.payload.operation === 'device.revoke' && v.payload.value.deviceId === c.deviceId,
          ),
      );
      await chain.verify(issuer.head.hash, c, this.head.ownerMember.signingKey);
      const prior = this.#authors.get(c.deviceId);
      requireValue(!prior || equal(prior.signingKey, c.signingKey));
      this.#authors.set(c.deviceId, c);
      verified.push(c);
    }
    return verified;
  }
  async wraps(values: unknown) {
    requireValue(Array.isArray(values) && values.length <= 512 && this.head !== null);
    let previous = 0n;
    for (const raw of values) {
      const envelope = wrap.Envelope.fromJson(binary(raw, 2048)),
        h = envelope.header();
      const epoch = decimal(h.epoch);
      requireValue(
        epoch >= previous &&
          h.space === this.space &&
          h.page === this.page &&
          decimal(h.membershipRevision) <= this.head.revision &&
          equal(h.signerKey, this.owner),
      );
      previous = epoch;
      requireValue(
        (h.recipientKind === 'device' && h.recipientId === this.registration.deviceId) ||
          (h.recipientKind === 'member' && h.recipientId === this.head.ownerMember.id),
      );
      await envelope.verifyOwner(this.owner);
      // A member wrap cannot be opened by a device key. Do not substitute its key or authority.
      if (
        h.recipientKind !== 'device' ||
        h.recipientId !== this.registration.deviceId ||
        h.epoch !== this.epoch
      )
        continue;
      const secret = await envelope.open(h, this.registration.keys.enc, this.owner);
      try {
        requireValue(this.root === null);
        // wrap.open returns copy(..., 32), satisfying the opaque root's import-length precondition.
        this.root = await crypto.subtle.importKey('raw', secret, 'HKDF', false, ['deriveBits']);
      } finally {
        secret.fill(0);
      }
    }
  }
  baseline(value: payload.Baseline): Uint8Array {
    requireValue(this.head !== null && value.pageId === this.page && value.epoch === this.epoch);
    const revision = decimal(value.membershipRevision);
    requireValue(revision <= this.head.revision);
    const signed = this.#log[Number(revision - 1n)];
    requireValue(signed?.payload.operation === 'epoch.advance');
    const expected = signed.payload.value.baseline;
    for (const field of Object.keys(expected) as (keyof payload.Baseline)[])
      requireValue(value[field] === expected[field]);
    return this.head.ownerMember.signingKey.slice();
  }
  author(device: string, revision: string): Uint8Array {
    const c = this.#authors.get(device);
    if (!this.head || decimal(revision) > this.head.revision || !c)
      throw new Error('Fresh membership catchup required');
    requireValue(
      c.issuerKind === 'member' &&
        c.issuerId === this.head.ownerMember.id &&
        decimal(c.membershipRevision) <= decimal(revision) &&
        c.issuedAt <= Date.now() &&
        c.expiresAt > Date.now() &&
        !this.#log.some(
          (v) => v.payload.operation === 'device.revoke' && v.payload.value.deviceId === device,
        ),
    );
    return c.signingKey.slice();
  }
  validatePage(sharing: string) {
    let epoch: string | undefined,
      mode = 'private';
    for (const { payload: p } of this.#log) {
      if (p.operation === 'page.share' && p.value.pageId === this.page) {
        epoch = p.value.epoch;
        mode = p.value.mode;
      }
      if (p.operation === 'epoch.advance' && p.value.pageId === this.page) epoch = p.value.epoch;
      if (
        (p.operation === 'page.delete' || p.operation === 'page.archive') &&
        p.value.pageId === this.page
      )
        throw new Error('Page unavailable');
    }
    requireValue(epoch === this.epoch && mode === sharing);
  }
}
