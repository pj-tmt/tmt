import { certificate, decimal, encodeBinary, equal, requireValue } from '@tmt/colab-client';
import type {
  SealedSyncPort,
  SyncCall,
  SyncValue,
} from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';
import { Admission } from './admission.js';
import {
  admitHostedMembership,
  requireHostedEpoch,
  type HostedMembershipPin,
} from './hosted-authority.js';
import { Catchup } from './catchup.js';
import { Fold } from './fold.js';
import type { OwnRecord } from './fold-protocol.js';
import { sealOwnUpdate, type OwnAuthor } from './own-seal.js';
import { publishHostedOwn, type HostedPublishResult } from './hosted-publish.js';
import { Objects } from './objects.js';
import { HostedDiscovery, type HostedRunLimit } from './hosted-discovery.js';
import {
  hostedAuthoritySet,
  hostedCheckpointSet,
  hostedLog,
  hostedOwnLog,
  type HostedOriginal,
  type HostedScope,
} from './hosted-documents.js';
import { hostedQuery } from './hosted-query.js';
import type { PageView } from './transport.js';

/** One page's verifier and own sender, with shared serialized state and joined shutdown. */
export class HostedPageReader {
  private readonly discovery: HostedDiscovery;
  private readonly objects: Objects;
  private readonly scope: HostedScope;
  private readonly fold: Pick<Fold, 'run' | 'close'>;
  private readonly knownWriters = new Set<string>();
  private started = false;
  private ownStreamInUse = false;
  private closed = false;
  private readonly controller = new AbortController();
  private changes: Promise<void> = Promise.resolve();
  private closing?: Promise<void>;
  private readonly writes = new Set<Promise<unknown>>();
  constructor(
    private readonly port: SealedSyncPort,
    private readonly admission: Admission,
    private readonly pin: HostedMembershipPin,
    private readonly limits: ReadonlyMap<string, HostedRunLimit>,
    fold?: Pick<Fold, 'run' | 'close'>,
    private readonly ownAuthor?: OwnAuthor,
  ) {
    requireHostedEpoch(admission.epoch);
    requireValue(admission.reader !== undefined);
    this.fold = fold ?? new Fold();
    this.scope = { space: admission.space, page: admission.page, epoch: admission.epoch };
    this.discovery = new HostedDiscovery(port, limits);
    this.objects = new Objects(admission);
  }
  refresh(call: SyncCall, resources: ReadonlySet<string>): Promise<PageView> {
    return this.run(() => this.refreshNow(call, resources));
  }
  private async refreshNow(call: SyncCall, resources: ReadonlySet<string>): Promise<PageView> {
    this.live(call);
    const a = this.admission;
    if (!this.started || resources.has('authority')) {
      await a.restore();
      await admitHostedMembership(a, await this.authority('membership', '', call), this.pin);
      // Signed policy, rather than the page row's hint, determines whether re-sharing is needed.
      for (const { payload } of a.statements())
        if (
          (payload.operation === 'page.share' || payload.operation === 'epoch.advance') &&
          payload.value.pageId === a.page
        )
          requireHostedEpoch(payload.value.epoch);
    }
    a.validateRead(['link', 'public']);
    this.live(call);
    if (!a.root) {
      const wraps = await this.authority('wrap', `link:${a.reader!.linkId}`, call);
      requireValue(wraps.length === 1);
      await a.wraps(wraps.map(encodeBinary));
      requireValue(a.root !== null);
    }
    const catchup = new Catchup(a, ['link', 'public'], this.objects, a.epoch, true);
    try {
      const head = a.head!;
      await catchup.admitValue({
        ...this.frame(),
        membershipHead: {
          revision: head.revision.toString(),
          statementHash: encodeBinary(head.hash),
          ownerKey: encodeBinary(a.owner),
          statements: [],
          more: false,
        },
        baseline: null,
        streams: [],
        more: true,
      });
      const checkpoints =
        !this.started || resources.has('checkpoints')
          ? await hostedCheckpointSet(await this.discover('checkpoints', {}, call), this.scope)
          : [];
      const checkpointPairs = paired(checkpoints),
        writers = new Set(checkpointPairs.map((pair) => pair[0].header.context.authorDevice));
      const logChanged = !this.started || resources.has('log');
      if (logChanged) {
        for (const document of await this.discover('log', {}, call)) {
          const original = await hostedLog(document, this.scope);
          writers.add(original.header.context.authorDevice);
          if (
            original.header.context.authorDevice === a.registration.deviceId &&
            original.publisherUid !== this.port.context.uid
          )
            this.ownStreamInUse = true;
        }
      }
      requireValue(writers.size <= 256);
      // A new writer's log/checkpoint wakes the page even when membership has not changed.
      // Fetch its owner-issued certificate through the same bounded discovery, without a timer.
      if ([...writers].some((writer) => !this.knownWriters.has(writer))) {
        // Discovery also carries nonwriting link-device proofs. Only observed authors enter
        // the existing strict member-chain verifier; never discard a referenced writer's chain.
        const chains = (await this.authority('chain', '', call))
          .map((bytes) => {
            const chain = certificate.Chain.fromJson(bytes);
            return { deviceId: chain.certificate().deviceId, chain: encodeBinary(bytes) };
          })
          .filter((chain) => writers.has(chain.deviceId));
        for (let index = 0; index < chains.length; index += 64)
          await a.chains(chains.slice(index, index + 64));
      }
      for (const pair of checkpointPairs) {
        const stream = pair[0].header.context.authorDevice,
          held = this.objects.head(stream).seq,
          cut = decimal(pair[0].header.context.streamSeq);
        if (held !== 0n) {
          if (cut > held) throw catchUpRequired();
          continue;
        }
        for (const original of pair) await this.admit(catchup, original, 'checkpoint', call);
      }
      // Every query begins at this reader's cryptographically admitted shared author-chain head.
      // Transport cursors remain local to hostedQuery until all returned originals are verified.
      for (const stream of writers) {
        const after = Number(this.objects.head(stream).seq);
        requireValue(Number.isSafeInteger(after));
        const limit = this.limits.get('log');
        requireValue(limit !== undefined);
        const documents = await hostedQuery(
          this.port,
          {
            resource: 'log',
            scope: { ...this.values(), stream: { stringValue: stream } },
            stream,
            cursor: { stream, seq: after },
          },
          limit,
          call,
        );
        for (const document of documents)
          await this.admit(catchup, await hostedLog(document, this.scope), 'update', call);
      }
      this.live(call);
      await catchup.admitValue({ ...this.frame(), streams: [], more: false });
      for (const update of catchup.checkpoints) {
        this.live(call);
        await this.fold.run({
          type: 'checkpoint',
          update: update.update,
          ...(update.namespace === 'own' ? { writer: update.writer } : {}),
        });
      }
      this.live(call);
      const projection = await this.fold.run({
        type: 'apply',
        updates: catchup.updates
          .filter((value) => value.namespace === 'content')
          .map((value) => value.update),
        own: catchup.updates
          .filter((value) => value.namespace === 'own')
          .map(({ writer, update }) => ({ writer, update })),
      });
      this.live(call);
      // A key-expiry or signed access change during the finite read cannot publish stale plaintext.
      a.validateRead(['link', 'public']);
      writers.forEach((writer) => this.knownWriters.add(writer));
      this.started = true;
      return { ...projection, ownData: this.objects.ownData };
    } finally {
      catchup.close();
    }
  }
  /** A single shared stream lock spans fresh catch-up, own preparation, create and readback. */
  submitOwn(
    records: readonly OwnRecord[],
    call: SyncCall,
    fits?: (updateBytes: number) => void,
  ): Promise<HostedPublishResult> {
    try {
      this.live(call);
    } catch (error) {
      return Promise.reject(error);
    }
    const frozen = structuredClone([...records]);
    if (!this.ownAuthor || !navigator.locks)
      return Promise.reject({ code: 'REMOTE_STATE_UNAVAILABLE' });
    const signal = AbortSignal.any([
      call.signal,
      this.controller.signal,
      AbortSignal.timeout(Math.max(1, Math.min(30_000, call.deadlineAtMs - Date.now()))),
    ]);
    const bound = { ...call, signal },
      a = this.admission;
    const key = JSON.stringify([
      'colab-hosted-own',
      this.port.context.config.projectId,
      this.port.context.uid,
      a.space,
      a.page,
      a.epoch,
      a.registration.deviceId,
    ]);
    let attempted = false,
      confirmed = 0;
    const work = Promise.resolve(
      navigator.locks.request(key, { signal }, () =>
        this.run(async () => {
          this.live(bound);
          await this.refreshNow(bound, new Set(['authority', 'log', 'checkpoints']));
          this.live(bound);
          // Rules bind a stream continuation to its predecessor uid. This observation grants nothing.
          if (this.ownStreamInUse) return { outcome: 'writer-in-use' as const, confirmed: 0 };
          await this.authorize(a.registration.deviceId, a.head!.revision.toString());
          const prepared = await this.fold.run({
            type: 'prepare-own',
            writer: a.registration.deviceId,
            records: frozen,
          });
          try {
            this.live(bound);
            fits?.(prepared.update.length);
            const entry = await sealOwnUpdate(
              a,
              this.objects.head(a.registration.deviceId),
              prepared.update,
              this.authorize,
            );
            this.live(bound);
            const document = await hostedOwnLog(
              entry,
              this.scope,
              this.port.context.uid,
              Math.min(a.reader!.expiresAt, Date.now() + 365 * 24 * 60 * 60 * 1000),
            );
            this.live(bound);
            await this.authorize(a.registration.deviceId, a.head!.revision.toString());
            attempted = true;
            const result = await publishHostedOwn(this.port, [document], bound);
            confirmed = result.confirmed;
            if (result.outcome === 'sent') {
              const update = await this.objects.admit(
                a.registration.deviceId,
                entry,
                'update',
                'own',
              );
              if (update) {
                try {
                  await this.fold.run({ type: 'apply', updates: [], own: [update] });
                } finally {
                  update.update.fill(0);
                }
              }
              this.knownWriters.add(a.registration.deviceId);
            }
            return result;
          } finally {
            prepared.update.fill(0);
          }
        }),
      ),
    ).catch((failure: unknown): HostedPublishResult => ({
      outcome: attempted ? 'unconfirmed' : 'refused',
      confirmed,
      failure: signal.aborted
        ? {
            code: 'REMOTE_STATE_UNAVAILABLE',
            state: call.signal.aborted || this.closed ? 'cancelled' : 'deadline',
          }
        : failure,
    }));
    this.writes.add(work);
    void work.then(
      () => this.writes.delete(work),
      () => this.writes.delete(work),
    );
    return work;
  }
  private readonly authorize: OwnAuthor = async (device, revision) => {
    try {
      return await this.ownAuthor!(device, revision);
    } catch {
      throw { code: 'REMOTE_SCOPE_DENIED' };
    }
  };
  close(): Promise<void> {
    if (this.closing) return this.closing;
    this.closed = true;
    this.controller.abort();
    this.fold.close();
    this.closing = Promise.allSettled([...this.writes, this.changes]).then(() =>
      this.admission.closeKeys(),
    );
    return this.closing;
  }
  private run<T>(action: () => Promise<T>): Promise<T> {
    const work = this.changes.then(action);
    this.changes = work.then(
      () => {},
      () => {},
    );
    return work;
  }
  private frame() {
    return { version: 1, type: 'catchup', ...this.scope };
  }
  private values(): Record<string, SyncValue> {
    return {
      space: { stringValue: this.scope.space },
      page: { stringValue: this.scope.page },
      epoch: { integerValue: this.scope.epoch },
    };
  }
  private async discover(resource: string, extra: Record<string, SyncValue>, call: SyncCall) {
    this.live(call);
    const mapping = this.port.context.resources.find((value) => value.name === resource)?.sync
      ?.query?.discovery;
    requireValue(mapping !== undefined && mapping.seqEquals === null);
    return this.discovery.run(
      { resource, shape: mapping.name, scope: { ...this.values(), ...extra } },
      call,
    );
  }
  private async authority(
    kind: 'membership' | 'chain' | 'wrap',
    recipient: string,
    call: SyncCall,
  ) {
    return hostedAuthoritySet(
      await this.discover(
        'authority',
        { kind: { stringValue: kind }, recipient: { stringValue: recipient } },
        call,
      ),
      this.scope,
      kind,
      recipient,
    );
  }
  private async admit(
    catchup: Catchup,
    original: HostedOriginal,
    kind: 'update' | 'checkpoint',
    call: SyncCall,
  ) {
    this.live(call);
    const context = original.header.context,
      entry = {
        seq: context.streamSeq,
        envelopeHash: encodeBinary(original.hash),
        envelope: encodeBinary(original.bytes),
      };
    await catchup.admitValue({
      ...this.frame(),
      streams: [
        {
          streamId: context.authorDevice,
          namespace: context.namespace,
          checkpoint: kind === 'checkpoint' ? entry : null,
          tail: kind === 'update' ? [entry] : [],
        },
      ],
      more: true,
    });
    this.live(call);
  }
  private live(call: SyncCall): void {
    if (this.closed || call.signal.aborted)
      throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' };
    if (!Number.isFinite(call.deadlineAtMs) || Date.now() >= call.deadlineAtMs)
      throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'deadline' };
  }
}

/** Each published prefix must have both namespaces, with one shared signed sequence/hash. */
function paired(originals: readonly HostedOriginal[]): readonly (readonly HostedOriginal[])[] {
  const streams = new Map<string, Map<bigint, Map<string, HostedOriginal>>>();
  for (const original of originals) {
    const c = original.header.context,
      seq = decimal(c.streamSeq),
      cuts = streams.get(c.authorDevice) ?? new Map<bigint, Map<string, HostedOriginal>>(),
      pair = cuts.get(seq) ?? new Map<string, HostedOriginal>();
    requireValue(!pair.has(c.namespace));
    pair.set(c.namespace, original);
    cuts.set(seq, pair);
    streams.set(c.authorDevice, cuts);
  }
  const selected: HostedOriginal[][] = [];
  for (const cuts of streams.values()) {
    const latest = [...cuts.keys()].reduce((a, b) => (a > b ? a : b)),
      pair = cuts.get(latest)!;
    const content = pair.get('content'),
      own = pair.get('own');
    if (!content || !own) throw catchUpRequired();
    requireValue(equal(content.header.context.prevHash, own.header.context.prevHash));
    selected.push([content, own]);
  }
  return selected;
}
const catchUpRequired = () => ({
  code: 'REMOTE_STATE_UNAVAILABLE' as const,
  state: 'catch-up-required' as const,
});
