/** Remote-owned page state. Presentation supplies controls, never authority. */
import type {
  RemoteManagement,
  ManagementOutcome,
  SettingChange,
  SettingsView,
  DevicePage,
  FirestoreSettingsView,
  RemovalPreview,
} from './management.js';
import { ClientError, RefusalError, ReopenSessionError } from 'remote-browser-sdk';

export type PageIntent =
  | { kind: 'setting'; input: SettingChange }
  | { kind: 'rename'; input: { operationId: string; clientId: string; name: string } }
  | { kind: 'talk'; input: { operationId: string; clientId: string; enabled: boolean } }
  | { kind: 'revoke'; input: { operationId: string; clientId: string } }
  | { kind: 'remove-others'; input: { operationId: string; fingerprint: string } };
export class ManagementPage {
  settings?: SettingsView;
  devices?: DevicePage;
  firestore?: FirestoreSettingsView;
  firestoreAccess: 'checking' | 'confirmed' | 'unconfirmed' = 'checking';
  intent?: Readonly<PageIntent>;
  outcome?: ManagementOutcome;
  access: 'checking' | 'live' | 'lost' | 'unconfirmed' = 'checking';
  accessRefusal?: InstanceType<typeof RefusalError>;
  busy = false;
  removalPreview?: RemovalPreview;
  get clientId(): string {
    return this.client.clientId;
  }
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
    return this.writableWhenIdle && !this.busy;
  }
  /** Whether controls are available at all; a busy window only fences them, it never mutes them. */
  get writableWhenIdle(): boolean {
    return (
      this.access === 'live' &&
      this.settings?.capabilities.settingsWrite === true &&
      this.outcome?.state !== 'unknown' &&
      !(this.outcome?.state === 'refused' && this.outcome.reason === 'REMOTE_MANAGEMENT_CAPACITY')
    );
  }
  async refresh(cursor: string | null = this.cursor): Promise<void> {
    if (this.busy || this.access === 'lost') return;
    this.busy = true;
    this.removalPreview = undefined;
    this.accessRefusal = undefined;
    this.firestoreRead = undefined;
    this.firestore = undefined;
    this.firestoreAccess = 'checking';
    const read = async () => {
      const settings = await this.client.settings();
      const devices = await this.client.devices({ cursor, limit: 25 });
      this.settings = settings;
      this.devices = devices;
      this.cursor = cursor;
      this.access = 'live';
      if (this.outcome) this.describeOutcome();
      else this.notice = settings.settings.warning ?? '';
    };
    try {
      try {
        await read();
      } catch (error) {
        if (
          !(error instanceof RefusalError) ||
          !['REMOTE_GRANT_CHANGED', 'REMOTE_SESSION_ENDED', 'REMOTE_SESSION_EVICTED'].includes(
            error.code,
          )
        )
          throw error;
        this.client = await this.reopen();
        await read();
      }
    } catch (error) {
      this.firestoreAccess = 'unconfirmed';
      this.failedAccess(error);
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
  async previewRemoval(): Promise<void> {
    if (!this.writable) return;
    this.busy = true;
    this.removalPreview = undefined;
    try {
      this.removalPreview = Object.freeze(await this.client.previewRemoveOthers());
    } catch {
      this.notice = 'Change refused. Check the current values or use the local CLI.';
    } finally {
      this.busy = false;
    }
  }
  async submit(intent: PageIntent): Promise<void> {
    if (!this.writable) throw new Error('Management controls are read-only.');
    this.removalPreview = undefined;
    // Retain the original input independently of subsequent form editing.
    this.intent = Object.freeze({
      ...intent,
      input: Object.freeze({ ...intent.input }),
    }) as PageIntent;
    this.outcome = undefined;
    this.freshAttempted = false;
    this.busy = true;
    // The pressed action's delayed mark is the only busy signal: no words while the write is in flight.
    this.notice = '';
    try {
      const frozen = this.intent;
      this.outcome =
        frozen.kind === 'setting'
          ? await this.client.set(frozen.input)
          : frozen.kind === 'rename'
            ? await this.client.rename(frozen.input)
            : frozen.kind === 'talk'
              ? await this.client.talk(frozen.input)
              : frozen.kind === 'remove-others'
                ? await this.client.removeOthers(frozen.input)
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
  /** One explicit bounded live-grant re-admission series, then only the original read.
   * Access refusal never establishes the prior mutation's outcome.
   */
  async recover(): Promise<void> {
    if (!this.intent || this.busy || this.freshAttempted) return;
    this.freshAttempted = true;
    this.accessRefusal = undefined;
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
      this.failedAccess(error);
      this.describeOutcome();
    } finally {
      this.busy = false;
    }
  }
  private failedAccess(error: unknown): void {
    this.accessRefusal =
      error instanceof ReopenSessionError && error.cause instanceof RefusalError
        ? error.cause
        : error instanceof RefusalError
          ? error
          : undefined;
    this.access = accessRefused(error) ? 'lost' : 'unconfirmed';
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
              : intent?.kind === 'remove-others'
                ? 'The other devices were removed. This browser stays connected.'
                : 'Device removed.';
      if (this.outcome.sessionEnded)
        this.notice += ' This session ended. Use the local CLI to continue.';
    } else if (this.outcome?.state === 'unknown')
      this.notice =
        this.intent?.kind === 'revoke' || this.intent?.kind === 'remove-others'
          ? 'Could not confirm the removal. Check its result before trying again.'
          : this.freshAttempted
            ? 'The original result is still unconfirmed. Check with tmt remote settings or tmt remote devices.'
            : 'This change could not be confirmed. Check its original result before making another change.';
    else if (this.outcome?.state === 'refused')
      this.notice =
        this.outcome.reason === 'REMOTE_DEVICE_LIST_CHANGED'
          ? 'The device list changed. Review it, then confirm again.'
          : this.outcome.reason === 'REMOTE_MANAGEMENT_CAPACITY'
            ? 'Browser change limit reached. Use the local CLI; do not retry or reset storage.'
            : this.outcome.reason === 'REMOTE_MANAGEMENT_READ_ONLY'
              ? 'This browser is read-only. Use the local CLI to make changes.'
              : 'Change refused. Check the current values or use the local CLI.';
  }
}

function accessRefused(error: unknown): boolean {
  if (error instanceof ReopenSessionError)
    return error.reason === 'revoked' || error.reason === 'expired';
  return (
    error instanceof RefusalError &&
    ['REMOTE_DEVICE_REVOKED', 'REMOTE_GRANT_EXPIRED'].includes(error.code)
  );
}
