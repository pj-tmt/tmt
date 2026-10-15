import { readFileSync } from 'node:fs';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import type {
  SealedDocument,
  SealedSyncPort,
  SyncCall,
  SyncMutation,
} from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';
import type { HostedBootstrap } from '../src/hosted-bootstrap.js';
import type { ReaderLink } from '../src/reader-link.js';
import { HostedEntry, type HostedEntryState } from '../src/hosted-entry.js';
import { Admission } from '../src/admission.js';
import type { OwnAuthor } from '../src/own-seal.js';
import type { OwnRecord } from '../src/fold-protocol.js';

const effects = vi.hoisted(() => [] as string[]);
const publication = vi.hoisted(() => ({
  author: undefined as OwnAuthor | undefined,
  device: '',
  records: [] as OwnRecord[],
  fits: false,
}));
afterEach(() => vi.restoreAllMocks());
vi.mock('../src/hosted-reader.js', () => ({
  HostedPageReader: class {
    constructor(
      readonly port: SealedSyncPort,
      admission: Admission,
      _link: unknown,
      _limits: unknown,
      _fold: unknown,
      author?: OwnAuthor,
    ) {
      publication.author = author;
      publication.device = admission.registration.deviceId;
    }
    async submitOwn(records: OwnRecord[], _call: SyncCall, fits?: (size: number) => void) {
      await publication.author!(publication.device, '1');
      fits?.(100);
      publication.fits = fits !== undefined;
      publication.records = records;
      return { outcome: 'sent', confirmed: 1 };
    }
    async refresh(call: SyncCall) {
      if (call.signal.aborted) throw new Error('cancelled');
      effects.push('verified');
      return { source: 'verified only', title: 'Page' };
    }
    close() {
      effects.push('reader-close');
    }
  },
}));
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
const hex = (value: string) =>
  Uint8Array.from(value.match(/../g) ?? [], (byte) => parseInt(byte, 16));
const target = (page = v.page): ReaderLink => ({
  space: v.space,
  page,
  link: v.linkId,
  revision: '1',
  statement: hex(v.statementHash),
  seed: hex(v.linkSeed),
  root: hex(v.public),
  role: 'editor',
});
const hash = async (bytes: Uint8Array) =>
  [...(await c.digest(bytes))].map((value) => value.toString(16).padStart(2, '0')).join('');
const pending = () => {
  let resolve!: () => void;
  const promise = new Promise<void>((yes) => {
    resolve = yes;
  });
  return { resolve, promise };
};
async function fixture() {
  effects.length = 0;
  publication.records = [];
  publication.fits = false;
  const states: HostedEntryState[] = [],
    creates: SyncMutation[] = [],
    ports: SealedSyncPort[] = [];
  const rows = new Map<string, SealedDocument>(),
    wakes: ((error: unknown) => void)[] = [];
  let active = 0,
    peak = 0,
    epoch = '1';
  const makePort = () => {
    const port = {
      context: {
        space: v.space,
        uid: 'holder-uid',
        extension: 'colab',
        declarationDigest: 'a'.repeat(64),
        config: { projectId: 'demo-example' },
      },
      async read(target: { resource: string; id: string }) {
        if (target.resource === 'members') return rows.get(target.id);
        const page = target.id.slice(v.space.length + 1);
        const bytes = c.text(
          JSON.stringify({
            space: { stringValue: v.space },
            page: { stringValue: page },
            epoch: { integerValue: epoch },
            state: { stringValue: 'open' },
            expiresAt: { timestampValue: '2027-01-01T00:00:00Z' },
          }),
        );
        return { target, bytes, digest: await hash(bytes) };
      },
      async publish(mutation: SyncMutation) {
        creates.push(mutation);
        if (mutation.mode !== 'create') throw new Error('no content writes');
        rows.set(mutation.document.target.id, mutation.document);
        return { kind: 'confirmed' };
      },
      async subscribe() {
        active++;
        peak = Math.max(active, peak);
        let reject: ((error: unknown) => void) | undefined,
          stopped = false;
        wakes.push((error) => reject?.(error));
        return {
          next(call: SyncCall) {
            return new Promise((_yes, no) => {
              reject = no;
              const abort = () => no({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' });
              call.signal.addEventListener('abort', abort, { once: true });
              if (call.signal.aborted) abort();
            });
          },
          close() {
            if (stopped) return;
            stopped = true;
            active--;
            reject?.({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' });
          },
        };
      },
      close() {
        expect(active).toBe(0);
        effects.push('port-close');
      },
    } as unknown as SealedSyncPort;
    ports.push(port);
    return port;
  };
  const boot = {
    origin: 'https://demo-example.web.app',
    declaration: { bytes: 'exact declaration', digest: 'a'.repeat(64) },
    limits: new Map(),
    auth: {
      signInAnonymous: vi.fn(async () => {
        effects.push('sign-in');
        return 'holder-uid';
      }),
      sealedSync: vi.fn(async () => makePort()),
      close: vi.fn(async () => {
        effects.push('auth-close');
      }),
    },
    budget: {
      snapshot: async () => ({ state: 'ok', reads: 10, writes: 1, day: 1 }),
      drain: vi.fn(async () => {
        effects.push('drain');
      }),
    },
  } as unknown as HostedBootstrap;
  const load = vi.fn(async () => {
    effects.push('bootstrap');
    return boot;
  });
  const entry = new HostedEntry({ state: (state) => states.push(state), bootstrap: load });
  return {
    entry,
    states,
    boot,
    load,
    creates,
    ports,
    wakes,
    active: () => active,
    peak: () => peak,
    epoch: (value: string) => {
      epoch = value;
    },
  };
}

it('fresh hosted entry is real sign-in with no preview, provider read or page interest', async () => {
  const f = await fixture();
  await f.entry.select();
  expect(f.states.at(-1)).toEqual({ phase: 'sign-in', hasLink: false });
  expect(f.boot.auth.signInAnonymous).not.toHaveBeenCalled();
  expect(f.ports).toEqual([]);
  await f.entry.signIn();
  expect(f.states.at(-1)).toEqual({ phase: 'link-needed' });
  expect(f.ports).toEqual([]);
  await f.entry.close();
  expect(f.boot.auth.close).toHaveBeenCalledOnce();
});

it('consumes Colab parsed keys before bootstrap, then one Rules admission and read-only group', async () => {
  const f = await fixture(),
    input = target();
  const selected = f.entry.select(input);
  expect(input.seed.some(Boolean)).toBe(false);
  await selected;
  expect(f.states.at(-1)).toEqual({ phase: 'sign-in', hasLink: true, role: 'editor' });
  await f.entry.signIn();
  expect(f.active()).toBe(3);
  expect(f.creates).toHaveLength(1);
  expect(f.creates[0]).toMatchObject({
    mode: 'create',
    document: { target: { resource: 'members' } },
  });
  const mutation = f.creates[0];
  if (mutation.mode !== 'create') throw new Error('wrong mutation');
  expect(JSON.parse(c.decodeText(mutation.document.bytes))).toMatchObject({
    role: { stringValue: 'editor' },
    token: { stringValue: v.linkRulesToken },
    seq: { integerValue: '1' },
  });
  expect(f.states.at(-1)).toMatchObject({
    phase: 'ready',
    view: { source: 'verified only' },
    budget: { state: 'ok' },
  });
  expect(f.load).toHaveBeenCalledOnce();
  await f.entry.close();
  expect(f.active()).toBe(0);
  expect(effects.indexOf('reader-close')).toBeLessThan(effects.indexOf('auth-close'));
});

it('the default editor wires fresh Colab ownAuthor and the one-document request-size hook', async () => {
  const author = vi.spyOn(Admission.prototype, 'ownAuthor').mockResolvedValue(new Uint8Array(32));
  const f = await fixture();
  try {
    await f.entry.select(target());
    await f.entry.signIn();
    const ready = f.states.at(-1);
    expect(publication.device).toBe(v.linkHostedDevice.id);
    expect(ready?.phase).toBe('ready');
    if (ready?.phase !== 'ready') throw new Error('expected a verified ready page');
    expect(ready.edit).toBeTypeOf('function');
    const source = '<main>A requested edit</main>';
    expect(await ready.edit!(source, 'verified only', () => {})).toEqual({ request: 'sent' });
    expect(author).toHaveBeenCalledExactlyOnceWith('1');
    expect(publication.fits).toBe(true);
    expect(publication.records).toHaveLength(1);
    expect(publication.records[0]).toMatchObject({
      root: 'intents',
      value: { version: 1, kind: 'edit-request', source },
    });
    const record = publication.records[0];
    expect(record.key).toBe((record.value as Record<string, unknown>).operationId);
    expect(f.creates).toHaveLength(1); // Only holder enrollment here: the request uses the reader.
  } finally {
    await f.entry.close();
  }
});

it('switches pages only after old cancel/join, without replacing the signed-in identity or declaration', async () => {
  const f = await fixture();
  await f.entry.select(target());
  await f.entry.signIn();
  const next = f.entry.select(target('00000000-0000-4000-8000-000000000456'));
  expect(f.active()).toBe(0);
  await next;
  expect(f.active()).toBe(3);
  expect(f.peak()).toBe(3);
  expect(f.boot.auth.signInAnonymous).toHaveBeenCalledOnce();
  expect(f.load).toHaveBeenCalledOnce();
  expect(f.creates).toHaveLength(2);
  await f.entry.close();
});

it('hidden pages have no interests and resume by fresh verified catch-up without re-creating the member', async () => {
  const f = await fixture();
  await f.entry.select(target());
  await f.entry.signIn();
  await f.entry.visible(false);
  expect(f.active()).toBe(0);
  expect(f.states.at(-1)).toEqual({ phase: 'paused' });
  await f.entry.visible(true);
  expect(f.active()).toBe(3);
  expect(f.peak()).toBe(3);
  expect(f.creates).toHaveLength(1);
  await f.entry.close();
});

it('a later-epoch hint refuses before a listener or content decoder is created', async () => {
  const f = await fixture();
  f.epoch('2');
  await f.entry.select(target());
  await f.entry.signIn();
  expect(f.states.at(-1)).toMatchObject({
    phase: 'failed',
    failure: { code: 'COLAB_HOSTED_RESHARE_REQUIRED', message: 'ask the owner to share it again' },
  });
  expect(f.active()).toBe(0);
  expect(effects).not.toContain('verified');
  await f.entry.close();
});

it('Rules/budget/rate refusal stays stopped across visibility events, with no new port or member create', async () => {
  const f = await fixture();
  await f.entry.select(target());
  await f.entry.signIn();
  const ended = pending();
  f.boot.budget.drain = async () => {
    ended.resolve();
  };
  f.wakes[0]({ code: 'REMOTE_BUDGET_EXHAUSTED', resetAtMs: 9999999999999, retryAfterMs: 1000 });
  await ended.promise;
  await f.entry.visible(false);
  await f.entry.visible(true);
  expect(f.active()).toBe(0);
  expect(f.ports).toHaveLength(1);
  expect(f.creates).toHaveLength(1);
  expect(f.states.at(-1)).toMatchObject({
    phase: 'failed',
    failure: { code: 'REMOTE_BUDGET_EXHAUSTED' },
  });
  await f.entry.close();
});
