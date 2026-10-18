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
import { EditTooLarge } from '../src/save.js';
import type { OwnAuthor } from '../src/own-seal.js';
import type { OwnRecord } from '../src/fold-protocol.js';

const effects = vi.hoisted(() => [] as string[]);
const publication = vi.hoisted(() => ({
  author: undefined as OwnAuthor | undefined,
  device: '',
  records: [] as OwnRecord[],
  fits: false,
  /** What the reader reports for the next request, and the size it asks the fit hook about. */
  outcome: 'sent' as 'sent' | 'refused' | 'unconfirmed' | 'writer-in-use',
  size: 100,
  /** The page text the next refresh shows. */
  source: 'verified only',
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
      publication.fits = fits !== undefined;
      publication.records = records;
      try {
        fits?.(publication.size);
      } catch (failure) {
        // The reader reports a request that does not fit as a refusal that carries the failure.
        return { outcome: 'refused', confirmed: 0, failure };
      }
      return { outcome: publication.outcome, confirmed: publication.outcome === 'sent' ? 1 : 0 };
    }
    async refresh(call: SyncCall) {
      if (call.signal.aborted) throw new Error('cancelled');
      effects.push('verified');
      return { source: publication.source, title: 'Page' };
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
  publication.outcome = 'sent';
  publication.size = 100;
  publication.source = 'verified only';
  const states: HostedEntryState[] = [],
    creates: SyncMutation[] = [],
    ports: SealedSyncPort[] = [];
  const rows = new Map<string, SealedDocument>(),
    wakes: ((error: unknown) => void)[] = [],
    changes: (() => void)[] = [];
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
            return new Promise((yes, no) => {
              reject = no;
              changes.push(() => yes({ kind: 'changed' }));
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
    /** A listener sees a change: the page is read again and, when it differs, shown again. */
    change: (source: string) => {
      publication.source = source;
      changes.splice(0).forEach((wake) => wake());
    },
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
    const answered = ready.edit!(source, 'verified only', () => {});
    f.change(source);
    // A confirmed request is answered by the page, as for a native link.
    expect(await answered).toEqual({ kind: 'applied', source });
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

it('passes a later-epoch routing hint to the baseline verifier without adding interests', async () => {
  const f = await fixture();
  f.epoch('2');
  await f.entry.select(target());
  await f.entry.signIn();
  expect(f.states.at(-1)).toMatchObject({ phase: 'ready', view: { source: 'verified only' } });
  expect(f.active()).toBe(3);
  expect(f.peak()).toBe(3);
  expect(effects).toContain('verified');
  await f.entry.close();
  expect(f.active()).toBe(0);
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

async function ready(f: Awaited<ReturnType<typeof fixture>>) {
  vi.spyOn(Admission.prototype, 'ownAuthor').mockResolvedValue(new Uint8Array(32));
  await f.entry.select(target());
  await f.entry.signIn();
  const state = f.states.at(-1);
  if (state?.phase !== 'ready' || !state.edit) throw new Error('expected an editor page');
  return state.edit;
}

it('a confirmed request is changed when the page shows other text, as for a native link', async () => {
  const f = await fixture();
  const edit = await ready(f);
  const answered = edit('<main>mine</main>', 'verified only', () => {});
  f.change('<main>merged elsewhere</main>');
  expect(await answered).toEqual({ kind: 'changed', source: '<main>merged elsewhere</main>' });
  await f.entry.close();
});

it('a request too large to send says how large, not that the link was refused', async () => {
  const f = await fixture();
  const edit = await ready(f);
  // Over what one request carries: refused before anything is prepared.
  await expect(
    edit('x'.repeat(c.editRequest.SOURCE_BYTES + 1), 'verified only', () => {}),
  ).rejects.toBeInstanceOf(EditTooLarge);
  expect(publication.records).toEqual([]);
  // Over what one document holds once prepared: the reader reports a refusal that carries it.
  publication.size = 500_000;
  await expect(edit('<main>big</main>', 'verified only', () => {})).rejects.toBeInstanceOf(
    EditTooLarge,
  );
  expect(f.creates).toHaveLength(1); // Only holder enrollment: nothing was published.
  await f.entry.close();
});

it.each([
  ['refused', 'refused'],
  ['unconfirmed', 'unconfirmed'],
  // An occupied ID is a race lost to another holder of the link: not sent, and not a standing block.
  ['writer-in-use', 'refused'],
] as const)('a %s request ends without waiting for the page, as %s', async (outcome, ended) => {
  const f = await fixture();
  const edit = await ready(f);
  publication.outcome = outcome;
  expect(await edit('<main>mine</main>', 'verified only', () => {})).toEqual({ request: ended });
  await f.entry.close();
});

it('a request that lost a race to another holder of the link can be sent again, and each send is one original', async () => {
  const f = await fixture();
  const edit = await ready(f);
  publication.outcome = 'writer-in-use';
  const sends: unknown[][] = [];
  const record = () => sends.push(publication.records);
  expect(await edit('<main>mine</main>', 'verified only', () => {})).toEqual({
    request: 'refused',
  });
  record();
  // Nothing sent it again by itself; the next deliberate Save is a new request at the new head.
  expect(sends).toHaveLength(1);
  publication.outcome = 'sent';
  const source = '<main>mine</main>';
  const answered = edit(source, 'verified only', () => {});
  f.change(source);
  expect(await answered).toEqual({ kind: 'applied', source });
  record();
  expect(sends).toHaveLength(2);
  await f.entry.close();
});

it('a request still waiting is answered by the page shown after the tab was hidden and shown', async () => {
  const f = await fixture();
  const edit = await ready(f);
  const source = '<main>mine</main>';
  const answered = edit(source, 'verified only', () => {});
  await vi.waitFor(() => expect(publication.records).toHaveLength(1)); // Sent; now waiting.
  await f.entry.visible(false);
  expect(f.active()).toBe(0);
  publication.source = source;
  await f.entry.visible(true);
  expect(await answered).toEqual({ kind: 'applied', source });
  await f.entry.close();
});

it('a request still waiting ends when the page is left or the entry closes', async () => {
  const f = await fixture();
  const edit = await ready(f);
  const left = edit('<main>mine</main>', 'verified only', () => {});
  const settled = left.then(
    () => 'answered',
    (error: Error) => error.message,
  );
  await vi.waitFor(() => expect(publication.records).toHaveLength(1));
  await f.entry.select(target('00000000-0000-4000-8000-000000000456'));
  expect(await settled).toBe('Page closed');
  const next = f.states.at(-1);
  if (next?.phase !== 'ready' || !next.edit) throw new Error('expected the next editor page');
  publication.records = [];
  const closed = next.edit('<main>again</main>', 'verified only', () => {});
  const ended = closed.then(
    () => 'answered',
    (error: Error) => error.message,
  );
  await vi.waitFor(() => expect(publication.records).toHaveLength(1));
  await f.entry.close();
  expect(await ended).toBe('Page closed');
});
