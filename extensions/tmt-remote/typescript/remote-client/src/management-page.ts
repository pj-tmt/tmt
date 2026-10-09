/** Remote-owned page state. Presentation supplies controls, never authority. */
import type {
  RemoteManagement,
  ManagementOutcome,
  SettingChange,
  SettingsView,
  DevicePage,
  FirestoreSettingsView,
} from './management.js';
import { ClientError, RefusalError } from 'remote-browser-sdk';

export type PageIntent =
  | { kind: 'setting'; input: SettingChange }
  | { kind: 'rename'; input: { operationId: string; clientId: string; name: string } }
  | { kind: 'talk'; input: { operationId: string; clientId: string; enabled: boolean } }
  | { kind: 'revoke'; input: { operationId: string; clientId: string } };
export class ManagementPage {
  settings?: SettingsView;
  devices?: DevicePage;
  firestore?: FirestoreSettingsView;
  firestoreAccess: 'checking' | 'confirmed' | 'unconfirmed' = 'checking';
  intent?: Readonly<PageIntent>;
  outcome?: ManagementOutcome;
  access: 'checking' | 'live' | 'lost' | 'unconfirmed' = 'checking';
  busy = false;
  notice = '';
  private freshAttempted = false;
  private cursor: string | null = null;
  get onFirstPage(): boolean {
    return this.cursor === null;
  }
  constructor(
    private client: RemoteManagement,
    private readonly reopen: () => Promise<RemoteManagement>,
  ) {}
  get writable(): boolean {
    return (
      this.access === 'live' &&
      this.settings?.capabilities.settingsWrite === true &&
      !this.busy &&
      this.outcome?.state !== 'unknown' &&
      !(this.outcome?.state === 'refused' && this.outcome.reason === 'REMOTE_MANAGEMENT_CAPACITY')
    );
  }
  async refresh(cursor: string | null = this.cursor): Promise<void> {
    this.busy = true;
    this.firestore = undefined;
    this.firestoreAccess = 'checking';
    try {
      const settings = await this.client.settings();
      const devices = await this.client.devices({ cursor, limit: 25 });
      this.settings = settings;
      this.devices = devices;
      this.cursor = cursor;
      this.access = 'live';
      if (this.outcome) this.describeOutcome();
      else this.notice = settings.settings.warning ?? '';
      // Optional observation cannot replace settings access or a frozen effect outcome.
      try {
        this.firestore = await this.client.settings({ firestore: true });
        this.firestoreAccess = 'confirmed';
      } catch {
        this.firestoreAccess = 'unconfirmed';
      }
    } catch (error) {
      this.firestoreAccess = 'unconfirmed';
      this.access = accessRefused(error) ? 'lost' : 'unconfirmed';
      if (this.outcome) this.describeOutcome();
      else this.notice = 'Current access could not be confirmed. Use the local CLI.';
    } finally {
      this.busy = false;
    }
  }
  async submit(intent: PageIntent): Promise<void> {
    if (!this.writable) throw new Error('Management controls are read-only.');
    // Retain the original input independently of subsequent form editing.
    this.intent = Object.freeze({
      ...intent,
      input: Object.freeze({ ...intent.input }),
    }) as PageIntent;
    this.outcome = undefined;
    this.freshAttempted = false;
    this.busy = true;
    this.notice = 'Saving…';
    try {
      const frozen = this.intent;
      this.outcome =
        frozen.kind === 'setting'
          ? await this.client.set(frozen.input)
          : frozen.kind === 'rename'
            ? await this.client.rename(frozen.input)
            : frozen.kind === 'talk'
              ? await this.client.talk(frozen.input)
              : await this.client.revoke(frozen.input);
      if (this.outcome.state === 'committed' && this.outcome.sessionEnded)
        this.access = 'unconfirmed';
      if (
        this.outcome.state === 'refused' &&
        this.outcome.reason === 'REMOTE_MANAGEMENT_READ_ONLY'
      ) {
        this.access = 'unconfirmed';
      }
      this.describeOutcome();
    } catch (error) {
      if (!(error instanceof ClientError)) throw error;
      this.outcome = {
        operationId: this.intent.input.operationId,
        state: 'unknown',
        reason: 'effect_outcome_unconfirmed',
      };
      this.notice = 'Outcome unknown. Read the original operation; do not submit it again.';
    } finally {
      this.busy = false;
    }
  }
  /** One explicit fresh live-grant admission attempt, then only the original read.
   * Access refusal never establishes the prior mutation's outcome.
   */
  async recover(): Promise<void> {
    if (!this.intent || this.busy || this.freshAttempted) return;
    this.freshAttempted = true;
    this.busy = true;
    try {
      this.client = await this.reopen();
      this.access = 'live';
      try {
        this.outcome = await this.client.operation(this.intent.input.operationId);
        this.describeOutcome();
      } catch {
        this.notice =
          'Original outcome unavailable or unconfirmed. Confirm with tmt remote devices or tmt remote settings.';
      }
    } catch (error) {
      this.access = accessRefused(error) ? 'lost' : 'unconfirmed';
      this.notice = accessRefused(error)
        ? 'Current access lost. Prior outcome remains unknown unless already acknowledged. Confirm with tmt remote devices.'
        : 'Current access unconfirmed. Prior outcome remains unknown unless already acknowledged. Confirm with the local CLI.';
    } finally {
      this.busy = false;
    }
  }
  get canRecover(): boolean {
    return (
      !!this.intent &&
      !this.busy &&
      !this.freshAttempted &&
      (this.outcome?.state === 'unknown' || this.access !== 'live')
    );
  }
  private describeOutcome(): void {
    if (this.outcome?.state === 'committed')
      this.notice =
        this.intent?.kind === 'setting'
          ? 'Saved. Session limits apply at the next session open.'
          : 'Device change committed.';
    else if (this.outcome?.state === 'unknown')
      this.notice = 'Outcome unknown. Read the original operation; do not submit it again.';
    else if (this.outcome?.state === 'refused')
      this.notice =
        this.outcome.reason === 'REMOTE_MANAGEMENT_CAPACITY'
          ? 'Browser management operation limit reached. Use tmt remote settings or tmt remote devices; do not retry or reset storage.'
          : this.outcome.reason === 'REMOTE_MANAGEMENT_READ_ONLY'
            ? 'This browser is read-only. Use the local CLI.'
            : 'Change refused. Check the admitted values or use the local CLI.';
  }
}

function accessRefused(error: unknown): boolean {
  return (
    error instanceof RefusalError &&
    ['REMOTE_CLOSED', 'REMOTE_SESSION_ENDED', 'REMOTE_SESSION_EVICTED'].includes(error.code)
  );
}
