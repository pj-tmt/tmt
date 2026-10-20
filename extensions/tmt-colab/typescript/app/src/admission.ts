import {
  attachment,
  binary,
  certificate,
  decimal,
  deriveSpaceId,
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

/** A read-only link device: wraps addressed to the link open its page key, and nothing about
 * the verified log is persisted, because a reader pins nothing in the owner app's storage. */
export interface ReaderSeat {
  linkId: string;
  /** The server-issued reader principal that hello names; it is not the link device's ID. */
  principal: string;
  expiresAt: number;
}

/** Content subset for the owner browser or a link reader. A transport head/hash is never log authority. */
export class Admission {
  head: statement.Head | null = null;
  #log: statement.Verified[] = [];
  #raw: string[] = [];
  #target: { revision: bigint; hash: Uint8Array } | null = null;
  root: CryptoKey | null = null;
  #roots = new Map<string, CryptoKey>();
  #authors = new Map<string, certificate.Certificate>();
  constructor(
    readonly space: string,
    readonly page: string,
    readonly epoch: string,
    readonly owner: Uint8Array,
    readonly registration: Registration,
    readonly reader?: ReaderSeat,
  ) {}
  /** Verified authority for parent management; never renderer data. */
  statements(): readonly statement.Verified[] {
    return this.#log.slice();
  }
  async restore() {
    if (this.reader) {
      requireValue((await deriveSpaceId(this.owner)) === this.space);
      return;
    }
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
    if (!this.reader)
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
  /** Historical issuer verification ignores current expiry, matching native
   * fold. Fresh author() and the actual caller validateRead() still enforce it.
   * A chain is verified against exactly the issuer kind it names: the owner member at the
   * statement it cites, or a link at the `link.add` statement that introduced it. The two
   * never stand in for each other. */
  async chains(values: unknown) {
    requireValue(Array.isArray(values) && values.length <= 64 && this.head !== null);
    const verified: certificate.Certificate[] = [];
    for (const value of values) {
      exactKeys(value, ['deviceId', 'chain']);
      const chain = certificate.Chain.fromJson(binary(value.chain, 16 * 1024));
      requireValue(chain.certificate().deviceId === value.deviceId);
      verified.push(await this.#admitChain(chain));
    }
    return verified;
  }
  async #admitChain(chain: certificate.Chain): Promise<certificate.Certificate> {
    requireValue(this.head !== null);
    const c = chain.certificate();
    requireValue(c.space === this.space && c.issuedAt <= Date.now());
    if (c.issuerKind === 'member') {
      const issuer = this.#log[Number(decimal(c.membershipRevision)) - 1];
      requireValue(c.issuerId === this.head.ownerMember.id && issuer !== undefined);
      await chain.verify(issuer.head.hash, c, this.head.ownerMember.signingKey);
    } else {
      // Native `verify_chain`: the issuer is the link recipient its `link.add` created, the
      // chain cites that statement, and its revision lies between that statement and the head.
      requireValue(c.issuerKind === 'link');
      const add = this.#linkAdd(c.issuerId);
      requireValue(
        add !== undefined &&
          decimal(c.membershipRevision) >= add.head.revision &&
          decimal(c.membershipRevision) <= this.head.revision,
      );
      await chain.verify(add.head.hash, c, binary(add.value.linkSignKey, 32, 32));
    }
    const prior = this.#authors.get(c.deviceId);
    requireValue(
      !prior ||
        (equal(prior.signingKey, c.signingKey) &&
          prior.issuerKind === c.issuerKind &&
          prior.issuerId === c.issuerId),
    );
    this.#authors.set(c.deviceId, c);
    return c;
  }
  /** The `link.add` statement that created a link, with its signed payload. */
  #linkAdd(linkId: string) {
    for (const { payload: p, head } of this.#log)
      if (p.operation === 'link.add' && p.value.linkId === linkId) return { head, value: p.value };
    return undefined;
  }
  /** What a link-issued device may write at `revision`, as native `open_stream_object` decides
   * it from the membership state at that revision: the link exists, covers this page, the page
   * is writable and link-shared in that state at the envelope's epoch, and the role is not a
   * viewer's (an editor's alone writes `content`). A removal at or before the revision is
   * refused with every other reduction, by `readAuthor`. */
  #linkWrites(
    c: certificate.Certificate,
    namespace: Context['namespace'],
    envelopeEpoch: string,
    revision: bigint,
  ) {
    const add = this.#linkAdd(c.issuerId);
    requireValue(
      add !== undefined &&
        add.head.revision <= revision &&
        decimal(c.membershipRevision) >= add.head.revision &&
        decimal(c.membershipRevision) <= revision &&
        add.value.pages.includes(this.page) &&
        add.value.role !== 'viewer' &&
        (namespace !== 'content' || add.value.role === 'editor'),
    );
    let epoch = 1n,
      linkMode = false,
      writable = true;
    for (const { payload: p, head } of this.#log) {
      if (head.revision > revision) break;
      if (p.operation === 'epoch.advance' && p.value.pageId === this.page)
        epoch = decimal(p.value.epoch);
      else if (p.operation === 'page.share' && p.value.pageId === this.page)
        linkMode = p.value.mode === 'link';
      else if (
        (p.operation === 'page.archive' || p.operation === 'page.delete') &&
        p.value.pageId === this.page
      )
        writable = false;
    }
    requireValue(linkMode && writable && epoch === decimal(envelopeEpoch));
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
      const own = this.reader
        ? h.recipientKind === 'link' && h.recipientId === this.reader.linkId
        : h.recipientKind === 'device' && h.recipientId === this.registration.deviceId;
      requireValue(
        own ||
          (!this.reader &&
            h.recipientKind === 'member' &&
            h.recipientId === this.head.ownerMember.id),
      );
      await envelope.verifyOwner(this.owner);
      // A member wrap cannot be opened by a device key. Do not substitute its key or authority.
      if (!own || epoch > decimal(this.epoch) || decimal(this.epoch) - epoch >= 64n) continue;
      const secret = await envelope.open(h, this.registration.keys.enc, this.owner);
      try {
        requireValue(!this.#roots.has(h.epoch) && (h.epoch !== this.epoch || this.root === null));
        // wrap.open returns copy(..., 32), satisfying the opaque root's import-length precondition.
        const root = await crypto.subtle.importKey('raw', secret, 'HKDF', false, ['deriveBits']);
        this.#roots.set(h.epoch, root);
        if (h.epoch === this.epoch) this.root = root;
      } finally {
        secret.fill(0);
      }
    }
  }
  baseline(value: payload.Baseline, epoch = this.epoch): Uint8Array {
    const requested = decimal(epoch),
      current = decimal(this.epoch);
    requireValue(requested <= current && current - requested < 64n);
    requireValue(this.head !== null && value.pageId === this.page && value.epoch === epoch);
    const revision = decimal(value.membershipRevision);
    requireValue(revision <= this.head.revision);
    const signed = this.#log[Number(revision - 1n)];
    requireValue(signed?.payload.operation === 'epoch.advance');
    const expected = signed.payload.value.baseline;
    for (const field of Object.keys(expected) as (keyof payload.Baseline)[])
      requireValue(value[field] === expected[field]);
    return this.head.ownerMember.signingKey.slice();
  }
  cuts(device?: string, epoch = this.epoch) {
    return this.#log.flatMap(({ payload: p, head }) => {
      // A removed link ends its devices' streams at the cuts the removal signed, as a device's
      // revocation ends its own; the cuts name the streams.
      if (p.operation !== 'device.revoke' && p.operation !== 'link.remove') return [];
      if (p.operation === 'device.revoke' && device !== undefined && p.value.deviceId !== device)
        return [];
      return p.value.cuts
        .filter((c) => c.pageId === this.page && c.epoch === epoch)
        .map((c) => ({ revision: head.revision, cut: streamCut.decode(binary(c.cut, 1024)) }))
        .filter(
          ({ cut }) =>
            p.operation === 'device.revoke' || device === undefined || cut.streamId === device,
        );
    });
  }
  /** The cut a reduction signed for `stream` in this page's `epoch` and `namespace`: its own, or
   * else the one a verified owner advance that sealed this exact epoch signed after `after` and
   * before the reduction. Missing both bounds is a refusal, never a default. */
  #signedCut(
    statement: statement.Verified,
    after: bigint,
    epoch: string,
    namespace: string,
    stream: string,
  ): payload.Cut | undefined {
    const p = statement.payload;
    if (p.operation !== 'device.revoke' && p.operation !== 'link.remove') return undefined;
    const matches = (cut: payload.Cut) =>
      cut.pageId === this.page &&
      cut.epoch === epoch &&
      cut.namespace === namespace &&
      streamCut.decode(binary(cut.cut, 1024)).streamId === stream;
    return (
      p.value.cuts.find(matches) ??
      this.#log
        .filter(
          (seal) => seal.head.revision > after && seal.head.revision < statement.head.revision,
        )
        .flatMap((seal) =>
          seal.payload.operation === 'epoch.advance' &&
          seal.payload.value.pageId === this.page &&
          decimal(seal.payload.value.epoch) === decimal(epoch) + 1n
            ? seal.payload.value.cuts
            : [],
        )
        .find(matches)
    );
  }
  /** Owner-member provenance: the device chain was issued by the owner member
   * and verified at its membership revision, and the device is not revoked.
   * A bridge's own envelopes are admitted (`readAuthor`) but never have it, so a
   * status record can only come from such a device. */
  ownerDevice(device: string): boolean {
    try {
      this.author(device, this.head!.revision.toString());
      return true;
    } catch {
      return false;
    }
  }
  /** The bridge a verified `bridge.add` named at `revision`, as native `open_object` finds it
   * in the membership state at the envelope's revision: the latest add for this machine,
   * with the page and epoch that state granted. Fresh publication is fenced separately by `validatePage`; archived content
   * can still authenticate its historical authors through read admission. */
  #bridge(device: string, revision: bigint) {
    let bridge: { signKey: Uint8Array; pages: readonly string[] } | undefined;
    let epoch = 1n;
    for (const { payload: p } of this.#log.slice(0, Number(revision))) {
      if (p.operation === 'bridge.add' && p.value.machineId === device)
        bridge = {
          signKey: binary(p.value.machineSignKey, 32, 32),
          pages: p.value.pages,
        };
      if (p.operation === 'epoch.advance' && p.value.pageId === this.page)
        epoch = decimal(p.value.epoch);
    }
    return bridge && { ...bridge, epoch };
  }
  /** The pinned key that signed an envelope and whether its author is an owner-member device.
   * Admission matches native: a stream with a device chain is an owner-member device, or a
   * link-issued one (`ownerDevice:false`) under `#linkWrites` and the signed cuts of a
   * `link.remove`; a stream
   * named by a `bridge.add` at the envelope's revision is a bridge, admitted for the `own`
   * namespace of its granted pages only, and never as an owner device. Both stop at the
   * device's signed `device.revoke` cut. */
  readAuthor(
    context: Context,
    envelopeHash: Uint8Array,
  ): { key: Uint8Array; ownerDevice: boolean } {
    const revision = decimal(context.membershipRevision);
    if (!this.head || revision > this.head.revision)
      throw new Error('Fresh membership catchup required');
    const bridge = this.#bridge(context.authorDevice, revision),
      c = bridge ? undefined : this.#authors.get(context.authorDevice);
    if (!bridge && !c) throw new Error('Fresh membership catchup required');
    if (bridge)
      requireValue(
        context.namespace === 'own' &&
          bridge.epoch === decimal(context.epoch) &&
          bridge.pages.includes(this.page),
      );
    else if (c!.issuerKind === 'link') {
      // A link-issued device is admitted by exactly the rules native applies to it, and the
      // member path above is never its fallback. `c.issuedAt` bounds it as a member's.
      requireValue(c!.issuedAt <= Date.now());
      this.#linkWrites(c!, context.namespace, context.epoch, revision);
    } else
      requireValue(
        c!.issuerKind === 'member' &&
          c!.issuerId === this.head.ownerMember.id &&
          decimal(c!.membershipRevision) <= revision &&
          c!.issuedAt <= Date.now(),
      );
    const linkId = c?.issuerKind === 'link' ? c.issuerId : undefined;
    for (const statement of this.#log) {
      // The author's own revocation, or the removal of the link that issued its chain, ends
      // its stream at the cut the statement signed.
      const p = statement.payload;
      if (
        !(p.operation === 'device.revoke' && p.value.deviceId === context.authorDevice) &&
        !(p.operation === 'link.remove' && linkId !== undefined && p.value.linkId === linkId)
      )
        continue;
      requireValue(revision < statement.head.revision);
      const wrapped = this.#signedCut(
        statement,
        revision,
        context.epoch,
        context.namespace,
        context.authorDevice,
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
    return bridge
      ? { key: bridge.signKey.slice(), ownerDevice: false }
      : { key: c!.signingKey.slice(), ownerDevice: c!.issuerKind === 'member' };
  }
  /** The pinned keys of an owner fold's signer and of the device whose stream it covers, once both
   * are owner-member devices and the fold is bound by what the owner signed (colab-v1, "Planned
   * owner fold"). It decides authority only; the anchor and the object's bytes are checked by the
   * caller.
   * - Both chains were issued by the owner member at or before the fold's revision. A bridge,
   *   a link-issued device or a device with no chain is refused.
   * - A signer that was ever revoked folds only what its `device.revoke` lists: an unlisted fold
   *   is refused whatever revision it cites, because a stolen key can backdate that revision and
   *   a pruned prefix cannot be rechecked. A listed one must still predate the revoke.
   * - A covered device that was revoked is folded only up to the `tailHeadSeq` of the cut its
   *   revoke signed, or of the one an owner advance sealed after the fold's revision, never past
   *   it (the bound `readAuthor` applies to an object at its revision). */
  readFold(
    context: Context,
    covered: string,
    envelopeHash: Uint8Array,
  ): { signer: Uint8Array; covered: Uint8Array } {
    const revision = decimal(context.membershipRevision);
    if (!this.head || revision > this.head.revision)
      throw new Error('Fresh membership catchup required');
    requireValue(
      context.kind === 'fold' &&
        !this.#bridge(context.authorDevice, revision) &&
        !this.#bridge(covered, revision),
    );
    const signer = this.#authors.get(context.authorDevice),
      target = this.#authors.get(covered);
    if (!signer || !target) throw new Error('Fresh membership catchup required');
    for (const c of [signer, target])
      requireValue(
        c.issuerKind === 'member' &&
          c.issuerId === this.head.ownerMember.id &&
          decimal(c.membershipRevision) <= revision &&
          c.issuedAt <= Date.now(),
      );
    const seq = decimal(context.streamSeq);
    for (const statement of this.#log) {
      const p = statement.payload;
      if (p.operation !== 'device.revoke') continue;
      if (p.value.deviceId === context.authorDevice)
        requireValue(
          revision < statement.head.revision && payload.listsFold(p.value, envelopeHash),
        );
      if (p.value.deviceId === covered) {
        // The same lower bound as `readAuthor`: an owner advance at or before the fold's own
        // revision sealed the epoch before the fold and cannot supply the cut.
        const cut = this.#signedCut(statement, revision, context.epoch, context.namespace, covered);
        requireValue(
          cut !== undefined &&
            seq <= decimal(streamCut.decode(binary(cut.cut, 1024)).tailHeadSeq, true),
        );
      }
    }
    return { signer: signer.signingKey.slice(), covered: target.signingKey.slice() };
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
  /** The signing key of a link-issued device that writes its own stream now: the fresh-write
   * counterpart of `author()` for a chain a link issued. It stands only while the chain is
   * unexpired, the device is not revoked, the link is not removed, and the link may write `own`
   * records to this page at `revision` and this admission's epoch (`#linkWrites`). A member's
   * chain never passes here, and a link's never passes `author()`. */
  async linkAuthor(device: string, revision: string): Promise<Uint8Array> {
    // The session's own chain is verified the first time it writes; streams the serve broadcasts
    // bring every other chain.
    if (!this.#authors.has(device) && device === this.registration.deviceId && this.head)
      await this.#admitChain(this.registration.chain);
    const c = this.#authors.get(device);
    if (!this.head || decimal(revision) > this.head.revision || !c)
      throw new Error('Fresh membership catchup required');
    requireValue(
      c.issuerKind === 'link' &&
        c.issuedAt <= Date.now() &&
        c.expiresAt > Date.now() &&
        !this.#log.some(
          ({ payload: p }) =>
            (p.operation === 'device.revoke' && p.value.deviceId === device) ||
            (p.operation === 'link.remove' && p.value.linkId === c.issuerId),
        ),
    );
    this.#linkWrites(c, 'own', this.epoch, decimal(revision));
    return c.signingKey.slice();
  }
  /** The signing key this admission's own device writes its `own` stream with now: the member
   * path (`author`) for a member's device, the link path (`linkAuthor`) for a link's. */
  async ownAuthor(revision: string): Promise<Uint8Array> {
    const device = this.registration.deviceId;
    return this.registration.chain.certificate().issuerKind === 'link'
      ? this.linkAuthor(device, revision)
      : this.author(device, revision);
  }
  /** Eligible keys came from this actual device/link's verified addressed wraps.
   * A member wrap or ambient server epoch secret never enters this map. */
  readRoot(epoch: string): CryptoKey {
    const current = decimal(this.epoch),
      requested = decimal(epoch);
    requireValue(requested <= current && current - requested < 64n);
    const root = epoch === this.epoch ? this.root : this.#roots.get(epoch);
    requireValue(root !== null && root !== undefined);
    return root;
  }
  closeKeys() {
    this.root = null;
    this.#roots.clear();
  }
  /** Asset sequence zero is deliberately not passed through readAuthor's
   * positive-stream cut check. The admitted own publication proves creation. */
  assetAuthor(descriptor: attachment.AttachmentDescriptor): Uint8Array {
    const d = attachment.attachmentDescriptor(descriptor),
      revision = decimal(d.membershipRevision),
      c = this.#authors.get(d.authorDevice);
    requireValue(
      this.head !== null &&
        revision <= this.head.revision &&
        c !== undefined &&
        d.space === this.space &&
        d.page === this.page &&
        c.issuerKind === 'member' &&
        c.issuerId === this.head.ownerMember.id &&
        decimal(c.membershipRevision) <= revision,
    );
    let epoch = '1';
    for (const { payload: p, head } of this.#log) {
      if (head.revision > revision) break;
      if (p.operation === 'epoch.advance' && p.value.pageId === this.page) epoch = p.value.epoch;
      if (p.operation === 'device.revoke' && p.value.deviceId === d.authorDevice)
        throw new Error('Attachment creator was revoked');
      if (
        (p.operation === 'page.delete' || p.operation === 'page.archive') &&
        p.value.pageId === this.page
      )
        throw new Error('Attachment creation was unavailable');
    }
    requireValue(epoch === d.epoch);
    return c.signingKey.slice();
  }
  /** Read-only presentation comes only from the verified owner log. */
  get archived(): boolean {
    return this.#log.some(
      ({ payload: p }) => p.operation === 'page.archive' && p.value.pageId === this.page,
    );
  }
  /** Current read policy, including the actual link seat. Writes still use
   * validatePage's writable default and author(), never this read purpose. */
  validateRead(sharing: string | readonly string[], epoch = this.epoch) {
    this.validatePage(sharing, true);
    const current = decimal(this.epoch),
      requested = decimal(epoch);
    requireValue(requested <= current && current - requested < 64n);
    if (!this.reader) {
      this.author(this.registration.deviceId, this.head!.revision.toString());
      return;
    }
    requireValue(Date.now() < this.reader.expiresAt);
    let active = false,
      currentOnly = false;
    for (const { payload: p } of this.#log) {
      if (p.operation === 'link.add' && p.value.linkId === this.reader.linkId)
        active = p.value.pages.includes(this.page);
      if (p.operation === 'link.remove' && p.value.linkId === this.reader.linkId) active = false;
      if (p.operation === 'page.history' && p.value.pageId === this.page)
        currentOnly = p.value.mode === 'current';
    }
    requireValue(active);
    requireValue(!currentOnly || epoch === this.epoch);
  }
  validatePage(sharing: string | readonly string[], reading = false) {
    let epoch: string | undefined,
      mode = 'private';
    for (const { payload: p } of this.#log) {
      if (p.operation === 'page.share' && p.value.pageId === this.page) {
        epoch = p.value.epoch;
        mode = p.value.mode;
      }
      if (p.operation === 'epoch.advance' && p.value.pageId === this.page) epoch = p.value.epoch;
      if (
        (p.operation === 'page.delete' || (!reading && p.operation === 'page.archive')) &&
        p.value.pageId === this.page
      )
        throw new Error('Page unavailable');
    }
    requireValue(epoch === this.epoch && [sharing].flat().includes(mode));
  }
}
