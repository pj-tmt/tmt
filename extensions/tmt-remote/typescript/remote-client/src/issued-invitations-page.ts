import type {
  InvitationCreationOutcome,
  InvitationIssueOutcome,
  InvitationOriginalOutcome,
  IssuedInvitationPage,
  RemoteInvitations,
} from './invitations.js';

type Intent =
  | Readonly<{ kind: 'create'; operationId: string }>
  | Readonly<{ kind: 'cancel'; operationId: string; offerId: string }>;

/** One view's observations and original effect. The signed service owns authority and storage. */
export class IssuedInvitationsPage {
  history?: IssuedInvitationPage;
  outcome?: InvitationIssueOutcome;
  secret?: Readonly<{ code: string; link: string | null }>;
  intent?: Intent;
  access: 'checking' | 'live' | 'unconfirmed' = 'checking';
  busy = false;
  private generation = 0;
  private disposed = false;
  private cursor: string | null = null;

  constructor(
    private client: RemoteInvitations,
    private readonly reopen: () => Promise<RemoteInvitations>,
  ) {}

  get onFirstPage(): boolean {
    return this.cursor === null;
  }

  get canRecover(): boolean {
    return !!this.intent && this.outcome?.state === 'unknown' && !this.busy && !this.disposed;
  }

  async refresh(cursor: string | null = this.cursor): Promise<void> {
    if (this.busy || this.disposed) return;
    const turn = ++this.generation;
    this.busy = true;
    try {
      const page = await this.client.list({ cursor });
      if (!this.current(turn)) return;
      this.history = page;
      this.cursor = cursor;
      this.access = 'live';
    } catch {
      if (this.current(turn)) {
        this.history = undefined;
        this.access = 'unconfirmed';
      }
    } finally {
      if (this.current(turn)) this.busy = false;
    }
  }

  async create(operationId: string): Promise<void> {
    await this.submit(Object.freeze({ kind: 'create', operationId }));
  }

  async cancel(operationId: string, offerId: string): Promise<void> {
    await this.submit(Object.freeze({ kind: 'cancel', operationId, offerId }));
  }

  private async submit(intent: Intent): Promise<void> {
    if (this.disposed || this.busy || this.access !== 'live' || this.outcome?.state === 'unknown')
      throw new Error('Invitation controls are unavailable.');
    this.secret = undefined;
    this.intent = intent;
    this.outcome = undefined;
    this.busy = true;
    const turn = ++this.generation;
    try {
      const result =
        intent.kind === 'create'
          ? await this.client.create({ operationId: intent.operationId })
          : await this.client.cancel({ operationId: intent.operationId, offerId: intent.offerId });
      if (!this.current(turn)) return;
      const creation = intent.kind === 'create' ? (result as InvitationCreationOutcome) : undefined;
      const createdSecret = creation?.state === 'committed' ? creation.secret : undefined;
      if (!this.matches(result, intent)) {
        this.outcome = this.unknown(intent.operationId);
        return;
      }
      if (intent.kind === 'create' && result.state === 'committed') {
        this.outcome = {
          operationId: result.operationId,
          state: result.state,
          sessionEnded: result.sessionEnded,
          result: result.result,
        };
        if (createdSecret) this.secret = Object.freeze({ ...createdSecret });
      } else this.outcome = result;
    } catch {
      if (this.current(turn)) this.outcome = this.unknown(intent.operationId);
    } finally {
      if (this.current(turn)) this.busy = false;
    }
  }

  /** Recovery reads the original metadata only; it never sends a new effect or recovers a secret. */
  async recover(): Promise<void> {
    if (!this.canRecover) return;
    const intent = this.intent!;
    this.secret = undefined;
    this.busy = true;
    const turn = ++this.generation;
    try {
      const client = await this.reopen();
      if (!this.current(turn)) return;
      this.client = client;
      const result = await client.operation(intent.operationId);
      if (!this.current(turn)) return;
      this.outcome = this.matches(result, intent) ? result : this.unknown(intent.operationId);
    } catch {
      if (this.current(turn)) this.access = 'unconfirmed';
    } finally {
      if (this.current(turn)) this.busy = false;
    }
  }

  /** The create result is shown only until its dialog is closed. */
  clearSecret(): void {
    this.secret = undefined;
  }

  dispose(): void {
    this.disposed = true;
    this.generation++;
    this.busy = false;
    this.clearSecret();
    this.history = undefined;
  }

  private current(turn: number): boolean {
    return !this.disposed && this.generation === turn;
  }

  private unknown(operationId: string): InvitationIssueOutcome {
    return { operationId, state: 'unknown', reason: 'effect_outcome_unconfirmed' };
  }

  private matches(
    result: InvitationOriginalOutcome,
    intent: Intent,
  ): result is InvitationIssueOutcome {
    if (result.operationId !== intent.operationId) return false;
    if (result.state !== 'committed') return true;
    const invitation = result.result.invitation;
    return intent.kind === 'create'
      ? invitation.state === 'created'
      : (invitation.state === 'cancelled' || invitation.state === 'not-active') &&
          invitation.offerId === intent.offerId;
  }
}
