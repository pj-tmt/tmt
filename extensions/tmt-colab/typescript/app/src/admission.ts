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
  streamCut,
  type Context,
  wrap,
} from '@tmt/colab-client';
import { record } from './storage.js';
import { verifyRegistration, type Registration } from './registration.js';

export const STATEMENT_ENVELOPE_BYTES = Math.floor(((payload.MAX_BYTES + 1024) * 4) / 3) + 2048;

/** Internal completed transfer; wire JSON cannot construct this value. */
export class StatementTransfer {
  constructor(
    readonly bytes: Uint8Array,
    readonly hash: Uint8Array,
  ) {}
}

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
  /** Verified authority for parent management; never renderer data. */
  statements(): readonly statement.Verified[] {
    return this.#log.slice();
  }
  async restore() {
    await verifyRegistration(this.registration, this.space, this.owner);
    const stored = (await record<string[]>(`log:${this.space}`)) ?? [];
    requireValue(Array.isArray(stored) && stored.every((raw) => typeof raw === 'string'));
    this.#budget(stored);
    const log: statement.Verified[] = [];
    let head: statement.Head | null = null;
    for (const raw of stored) {
      const entry = await this.#verify(raw, head);
      log.push(entry.verified);
      head = entry.verified.head;
    }
    this.head = head;
    this.#log = log;
    this.#raw = stored;
    this.#authors.set(this.registration.deviceId, this.registration.chain.certificate());
  }
  async #verify(raw: string | StatementTransfer, head: statement.Head | null) {
    const bytes =
        typeof raw === 'string' ? binary(raw, STATEMENT_ENVELOPE_BYTES) : raw.bytes.slice(),
      hash = typeof raw === 'string' ? null : raw.hash.slice(),
      envelope = statement.Envelope.fromJson(bytes);
    if (hash) requireValue(equal(await envelope.hash(), hash));
    const verified = await envelope.verifyNext(this.space, this.owner, head);
    return { verified, raw: encodeBinary(bytes) };
  }
  #budget(raw: string[]) {
    requireValue(
      raw.length <= 4096 && raw.reduce((n, value) => n + value.length, 0) <= 4 * 1024 * 1024,
    );
  }
  async membership(value: unknown, first: boolean) {
    exactKeys(
      value,
      first
        ? ['revision', 'statementHash', 'ownerKey', 'statements', 'more']
        : ['statements', 'more'],
    );
    let target = this.#target;
    if (first) {
      requireValue(
        equal(binary(value.ownerKey, 32, 32), this.owner) && typeof value.revision === 'string',
      );
      const revision = decimal(value.revision),
        hash = binary(value.statementHash, 32, 32);
      requireValue(!this.head || revision >= this.head.revision);
      if (this.head?.revision === revision) requireValue(equal(hash, this.head.hash));
      target = { revision, hash };
    }
    requireValue(
      target !== null &&
        Array.isArray(value.statements) &&
        value.statements.length <= 64 &&
        typeof value.more === 'boolean',
    );
    let head = this.head;
    const log = [...this.#log],
      raw = [...this.#raw];
    for (const entry of value.statements) {
      requireValue(
        typeof entry === 'string' ||
          (entry instanceof StatementTransfer && value.statements.length === 1),
      );
      if (typeof entry === 'string') binary(entry, 32 * 1024);
      const next = await this.#verify(entry, head);
      head = next.verified.head;
      log.push(next.verified);
      raw.push(next.raw);
    }
    requireValue(head !== null && head.revision <= target.revision);
    requireValue(
      value.more
        ? head.revision < target.revision
        : head.revision === target.revision && equal(head.hash, target.hash),
    );
    this.#budget(raw);
    await navigator.locks.request(`colab-log:${this.space}`, async () => {
      const previous = await record<string[]>(`log:${this.space}`);
      // Another tab may have advanced. Never replace its higher/forked durable head.
      if (previous && previous.length > raw.length) {
        requireValue(raw.every((v, i) => previous[i] === v));
      } else {
        requireValue(!previous || previous.every((v, i) => raw[i] === v));
        await record(`log:${this.space}`, raw);
      }
    });
    // Publish only after every check and durable transaction completes.
    this.head = head;
    this.#log = log;
    this.#raw = raw;
    this.#target = target;
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
          c.expiresAt > Date.now(),
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
  cuts(device?: string) {
    return this.#log.flatMap((v) =>
      v.payload.operation === 'device.revoke' &&
      (device === undefined || v.payload.value.deviceId === device)
        ? v.payload.value.cuts
            .filter((c) => c.pageId === this.page && c.epoch === this.epoch)
            .map((c) => ({ revision: v.head.revision, cut: streamCut.decode(binary(c.cut, 1024)) }))
        : [],
    );
  }
  readAuthor(context: Context, envelopeHash: Uint8Array): Uint8Array {
    const revision = decimal(context.membershipRevision);
    const c = this.#authors.get(context.authorDevice);
    if (!this.head || revision > this.head.revision || !c)
      throw new Error('Fresh membership catchup required');
    requireValue(
      c.issuerKind === 'member' &&
        c.issuerId === this.head.ownerMember.id &&
        decimal(c.membershipRevision) <= revision &&
        c.issuedAt <= Date.now() &&
        c.expiresAt > Date.now(),
    );
    for (const statement of this.#log) {
      if (
        statement.payload.operation !== 'device.revoke' ||
        statement.payload.value.deviceId !== context.authorDevice
      )
        continue;
      requireValue(revision < statement.head.revision);
      const wrapped = statement.payload.value.cuts.find(
        (c) =>
          c.pageId === this.page &&
          c.epoch === this.epoch &&
          c.namespace === context.namespace &&
          streamCut.decode(binary(c.cut, 1024)).streamId === context.authorDevice,
      );
      requireValue(wrapped !== undefined);
      const cut = streamCut.decode(binary(wrapped.cut, 1024)),
        seq = decimal(context.streamSeq);
      requireValue(seq <= decimal(cut.tailHeadSeq, true));
      if (context.kind === 'checkpoint')
        requireValue(
          context.streamSeq === cut.checkpointSeq &&
            cut.checkpointHash !== null &&
            equal(cut.checkpointHash, envelopeHash),
        );
      else if (context.streamSeq === cut.tailHeadSeq)
        requireValue(equal(envelopeHash, cut.tailHeadHash));
    }
    return c.signingKey.slice();
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
