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
  private firestoreRead?: Promise<FirestoreSettingsView>;
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
    this.firestoreRead = undefined;
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
    } catch (error) {
      this.firestoreAccess = 'unconfirmed';
      this.access = accessRefused(error) ? 'lost' : 'unconfirmed';
      if (this.outcome) this.describeOutcome();
      else this.notice = 'Current access could not be confirmed. Use the local CLI.';
    } finally {
      this.busy = false;
    }
  }
  /** Optional serialized observation owns no management busy window. */
  async observeFirestore(): Promise<void> {
    const read = this.client.settings({ firestore: true });
    this.firestoreRead = read;
    try {
      const view = await read;
      if (this.firestoreRead !== read) return;
      this.firestore = view;
      this.firestoreAccess = 'confirmed';
    } catch {
      if (this.firestoreRead !== read) return;
      this.firestoreAccess = 'unconfirmed';
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
    this.notice =
      intent.kind === 'revoke'
        ? 'Revoking device…'
        : intent.kind === 'talk'
          ? intent.input.enabled
            ? 'Enabling sending…'
            : 'Disabling sending…'
          : 'Saving…';
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
      this.describeOutcome();
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
        this.describeOutcome();
      }
    } catch (error) {
      this.access = accessRefused(error) ? 'lost' : 'unconfirmed';
      this.describeOutcome();
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
    if (this.outcome?.state === 'committed') {
      const intent = this.intent;
      this.notice =
        intent?.kind === 'setting'
          ? intent.input.setting === 'open'
            ? 'Browser opening saved.'
            : 'Session limit saved. Applies to new sessions.'
          : intent?.kind === 'rename'
            ? 'Device renamed.'
            : intent?.kind === 'talk'
              ? intent.input.enabled
                ? 'Sending enabled for this device.'
                : 'Sending disabled for this device.'
              : 'Device revoked.';
      if (this.outcome.sessionEnded)
        this.notice += ' This session ended. Use the local CLI to continue.';
    } else if (this.outcome?.state === 'unknown')
      this.notice = this.freshAttempted
        ? 'The original result is still unconfirmed. Check with tmt remote settings or tmt remote devices.'
        : 'This change could not be confirmed. Check its original result before making another change.';
    else if (this.outcome?.state === 'refused')
      this.notice =
        this.outcome.reason === 'REMOTE_MANAGEMENT_CAPACITY'
          ? 'Browser change limit reached. Use the local CLI; do not retry or reset storage.'
          : this.outcome.reason === 'REMOTE_MANAGEMENT_READ_ONLY'
            ? 'This browser is read-only. Use the local CLI to make changes.'
            : 'Change refused. Check the current values or use the local CLI.';
  }
}

function accessRefused(error: unknown): boolean {
  return (
    error instanceof RefusalError &&
    ['REMOTE_CLOSED', 'REMOTE_SESSION_ENDED', 'REMOTE_SESSION_EVICTED'].includes(error.code)
  );
}
