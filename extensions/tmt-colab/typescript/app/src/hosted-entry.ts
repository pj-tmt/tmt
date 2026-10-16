import { certificate, requireValue } from '@tmt/colab-client';
import type { SyncCall } from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';
import { Admission } from './admission.js';
import { HostedAdmission } from './hosted-admission.js';
import { requireHostedEpoch } from './hosted-authority.js';
import { bootstrapHosted, type HostedBootstrap } from './hosted-bootstrap.js';
import type { HostedBudget } from './hosted-budget.js';
import { HostedPageReader } from './hosted-reader.js';
import { HostedSession } from './hosted-session.js';
import { deriveReaderKeys } from './reader-keys.js';
import type { ReaderLink } from './reader-link.js';
import type { Registration } from './registration.js';
import type { PageView } from './transport.js';
import type { EditRequestOutcome, ReaderEdit } from './reader-source.js';
import { PageAnswer, sendEditRequest } from './link-edit.js';
import { EditTooLarge } from './save.js';

type Usage = Awaited<ReturnType<HostedBudget['snapshot']>>;
export type HostedEntryState =
  | { phase: 'loading' | 'link-needed' | 'recovering' | 'paused' }
  | { phase: 'sign-in'; hasLink: boolean; role?: ReaderLink['role'] }
  | { phase: 'ready'; view: PageView; budget: Usage; role?: ReaderLink['role']; edit?: ReaderEdit }
  | { phase: 'failed'; failure: unknown };
interface Options {
  state(value: HostedEntryState): void;
  bootstrap?(call: SyncCall): Promise<HostedBootstrap>;
  visible?: boolean;
}
interface Page {
  target: ReaderLink;
  keys: Awaited<ReturnType<typeof deriveReaderKeys>>;
  admission: HostedAdmission;
  /** The page text last shown for this page, across hidden and recovered sessions: how a sent
   * edit request is answered. Ended when the page is left. */
  answer: PageAnswer;
}

/** One origin/auth/declaration owner with one selected page. No native door fallback. */
export class HostedEntry {
  private readonly controller = new AbortController();
  private boot?: Promise<HostedBootstrap>;
  private changes = Promise.resolve();
  private retiring = Promise.resolve();
  private generation = 0;
  private page?: Page;
  private session?: HostedSession;
  private signedIn?: string;
  private signing?: Promise<void>;
  private terminal?: { failure: unknown };
  private shown: boolean;
  constructor(private readonly options: Options) {
    this.shown = options.visible ?? true;
  }

  /** The Colab parser owns the fragment. Fence the old page while an async parse completes. */
  select(selection?: ReaderLink | Promise<ReaderLink | undefined>): Promise<void> {
    const generation = ++this.generation,
      retiring = this.retire();
    this.page?.answer.end();
    this.options.state({ phase: 'loading' });
    const prepare = async (target?: ReaderLink): Promise<Page | undefined> =>
      target
        ? {
            target,
            keys: await deriveReaderKeys(target, 'hosted'),
            admission: new HostedAdmission(),
            answer: new PageAnswer(),
          }
        : undefined;
    // Direct parsed inputs consume their seed synchronously; queued parsing failures are handled now.
    const preparing = (
      selection instanceof Promise ? selection.then(prepare) : prepare(selection)
    ).then(
      (value) => ({ value }),
      (failure) => ({ failure }),
    );
    return this.enqueue(generation, async () => {
      await retiring;
      const prepared = await preparing;
      if (!this.current(generation)) return;
      if ('failure' in prepared) throw prepared.failure;
      this.page = prepared.value;
      this.terminal = undefined;
      await this.open(generation);
    });
  }
  signIn(): Promise<void> {
    if (this.signing) return this.signing;
    const generation = this.generation;
    const signing = this.enqueue(generation, async () => {
      if (!this.current(generation) || this.terminal) return;
      const boot = await this.bootstrap();
      if (!this.current(generation)) return;
      this.options.state({ phase: 'loading' });
      this.signedIn = await boot.auth.signInAnonymous();
      if (this.current(generation)) await this.open(generation);
    });
    this.signing = signing;
    void signing.then(() => {
      if (this.signing === signing) this.signing = undefined;
    });
    return signing;
  }
  visible(shown: boolean): Promise<void> {
    this.shown = shown;
    const generation = this.generation,
      retiring = shown ? this.retiring : this.retire();
    return this.enqueue(generation, async () => {
      await retiring;
      await this.open(generation);
    });
  }
  async close(): Promise<void> {
    ++this.generation;
    this.controller.abort();
    const retiring = this.retire();
    this.page?.answer.end();
    await this.changes;
    await retiring;
    const boot = await this.boot?.catch(() => undefined);
    if (boot) {
      await boot.budget.drain();
      await boot.auth.close();
    }
    this.page = undefined;
  }
  private current(generation: number): boolean {
    return !this.controller.signal.aborted && this.generation === generation;
  }
  private enqueue(generation: number, action: () => Promise<void>): Promise<void> {
    const work = this.changes.then(action).catch((failure) => {
      if (this.current(generation)) {
        this.terminal = { failure };
        this.options.state({ phase: 'failed', failure });
      }
    });
    this.changes = work;
    return work;
  }
  private bootstrap(): Promise<HostedBootstrap> {
    this.boot ??= (this.options.bootstrap ?? bootstrapHosted)({
      signal: this.controller.signal,
      deadlineAtMs: Date.now() + 30_000,
    });
    return this.boot;
  }
  private retire(): Promise<void> {
    const session = this.session;
    this.session = undefined;
    if (session) this.retiring = Promise.all([this.retiring, session.close()]).then(() => {});
    return this.retiring;
  }
  private async open(generation: number): Promise<void> {
    if (!this.current(generation)) return;
    if (this.terminal) {
      this.options.state({ phase: 'failed', failure: this.terminal.failure });
      return;
    }
    if (!this.shown) {
      this.options.state({ phase: 'paused' });
      return;
    }
    const boot = await this.bootstrap();
    if (!this.current(generation)) return;
    if (!this.shown) {
      this.options.state({ phase: 'paused' });
      return;
    }
    if (!this.signedIn) {
      this.options.state({
        phase: 'sign-in',
        hasLink: this.page !== undefined,
        ...(this.page ? { role: this.page.target.role } : {}),
      });
      return;
    }
    const page = this.page;
    if (!page) {
      this.options.state({ phase: 'link-needed' });
      return;
    }
    if (this.session) return;
    const { target, keys } = page;
    let reader: HostedPageReader | undefined;
    const { answer } = page;
    const registration: Registration = {
      deviceId: keys.device.id,
      chain: certificate.Chain.fromJson(keys.chain),
      keys: {
        sign: keys.device.sign,
        signPublic: keys.device.signingPublic,
        enc: keys.link.encryption,
      },
    };
    this.options.state({ phase: 'loading' });
    const session = new HostedSession(
      {
        open: async (call) => {
          const port = await boot.auth.sealedSync(target.space, boot.budget);
          try {
            requireValue(
              port.context.space === target.space &&
                port.context.uid === this.signedIn &&
                port.context.extension === 'colab' &&
                port.context.declarationDigest === boot.declaration.digest &&
                `https://${port.context.config.projectId}.web.app` === boot.origin,
            );
            const hints = await page.admission.open(
              port,
              {
                space: target.space,
                page: target.page,
                link: target.link,
                role: target.role,
                token: keys.link.rulesToken,
              },
              call,
            );
            requireHostedEpoch(hints.epoch);
            const admission = new Admission(
              target.space,
              target.page,
              hints.epoch,
              target.root,
              registration,
              {
                linkId: target.link,
                principal: port.context.uid,
                expiresAt: hints.expiresAtMs,
              },
            );
            reader = new HostedPageReader(
              port,
              admission,
              target,
              boot.limits,
              undefined,
              (_device, revision) => admission.ownAuthor(revision),
            );
            const scope = {
              space: { stringValue: target.space },
              page: { stringValue: target.page },
              epoch: { integerValue: hints.epoch },
            };
            return {
              port,
              reader,
              interests: [
                { resource: 'log', scope },
                { resource: 'checkpoints', scope },
                {
                  resource: 'authority',
                  scope: {
                    ...scope,
                    kind: { stringValue: 'membership' },
                    recipient: { stringValue: '' },
                  },
                },
              ],
            };
          } catch (error) {
            port.close();
            throw error;
          }
        },
        drain: () => boot.budget.drain(),
        publish: async (view, call) => {
          const budget = await boot.budget.snapshot();
          if (this.current(generation) && this.shown && !call.signal.aborted) {
            answer.show(view.source);
            this.options.state({
              phase: 'ready',
              view,
              budget,
              role: target.role,
              ...(target.role === 'editor' && reader
                ? {
                    edit: (source, base, onWaiting) =>
                      this.edit(reader!, generation, call.signal, answer, source, base, onWaiting),
                  }
                : {}),
            });
          }
        },
        recovering: () => {
          if (this.current(generation) && this.shown) this.options.state({ phase: 'recovering' });
        },
        failed: (failure) => {
          if (!this.current(generation)) return;
          this.terminal = { failure };
          answer.end();
          this.options.state({ phase: 'failed', failure });
        },
      },
      { signal: this.controller.signal, deadlineAtMs: Date.now() + 30_000 },
    );
    this.session = session;
    await session.ready.catch(() => {});
  }
  private async edit(
    reader: HostedPageReader,
    generation: number,
    signal: AbortSignal,
    answer: PageAnswer,
    source: string,
    base: string,
    onWaiting: () => void,
  ): ReturnType<ReaderEdit> {
    const before = answer.shown;
    const sent = await this.send(reader, generation, signal, source, base);
    // Once the request is confirmed, the page is the only answer, as for a native link.
    return sent === 'sent' ? answer.answer(source, before, onWaiting) : { request: sent };
  }
  /** One request through the reader: how it ended, or `EditTooLarge` when it cannot be one document. */
  private async send(
    reader: HostedPageReader,
    generation: number,
    signal: AbortSignal,
    source: string,
    base: string,
  ): Promise<'sent' | EditRequestOutcome> {
    if (!this.current(generation) || !this.shown || this.terminal || signal.aborted)
      return 'refused';
    let submitted = false;
    try {
      let outcome: 'sent' | EditRequestOutcome = 'refused';
      await sendEditRequest(
        {
          submitOwnRecords: async (records, fits) => {
            if (!this.current(generation) || signal.aborted) return;
            submitted = true;
            const result = await reader.submitOwn(
              records,
              { signal, deadlineAtMs: Date.now() + 30_000 },
              fits,
            );
            // The reader reports a request that does not fit as a refusal; the page names its size.
            if (result.failure instanceof EditTooLarge) throw result.failure;
            // An occupied ID is a race lost to another holder of the link (the Rules admit one
            // uid after another, and holders are not told apart): nothing of ours was stored, so
            // it is not sent, and the next deliberate Save catches up and sends at the new head.
            outcome = result.outcome === 'writer-in-use' ? 'refused' : result.outcome;
          },
        },
        base,
        source,
      );
      return outcome;
    } catch (failure) {
      if (failure instanceof EditTooLarge) throw failure;
      const code =
        typeof failure === 'object' && failure !== null && 'code' in failure
          ? failure.code
          : undefined;
      return !submitted ||
        code === 'REMOTE_SCOPE_DENIED' ||
        code === 'REMOTE_BUDGET_EXHAUSTED' ||
        code === 'REMOTE_RATE_LIMITED' ||
        code === 'REMOTE_INPUT_INVALID'
        ? 'refused'
        : 'unconfirmed';
    }
  }
}
