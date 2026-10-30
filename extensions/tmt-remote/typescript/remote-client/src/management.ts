import { managementMutation } from './management-mutation.js';
import { valid, input, object, exact, integer } from './management-codec.js';
import { parseFirestoreLayers, type FirestoreLayerView } from './firestore.js';
import { parsePublishedLimits, type FirestoreBudgetView } from './budget.js';
import type { Session } from './device.js';
import { channelFor } from './session-channel.js';
import { verifiedSessionRequest, type RemoteRefusalCode } from './operations.js';

export interface RemoteSettings {
  open: boolean;
  source: 'default' | 'settings.json';
  sessionsPerDevice: string | null;
  sessionsPerDeviceSource: 'default' | 'settings.json';
  warning: string | null;
}
export interface ManagementDevice {
  clientId: string;
  name: string;
  kind: 'browser' | 'addon' | 'cli';
  issuedAtMs: number;
  expiresAtMs: number | null;
  revision: number;
  revoked: boolean;
  talkEnabled: boolean;
}
export interface DevicePage {
  devices: (ManagementDevice & {
    thisBrowser: boolean;
    liveSessionCount: number;
    lastActivityAtMs: number | null;
    lastUsedAtMs: number | null;
  })[];
  nextCursor: string | null;
}
export interface SettingsView {
  settings: RemoteSettings;
  capabilities: { settingsWrite: boolean; devicesWrite: boolean };
  readOnlyReason: 'local_cli_required' | null;
}
export interface FirestoreSettingsView extends SettingsView {
  firestoreLayers: FirestoreLayerView[];
  firestoreBudget: FirestoreBudgetView;
}
export type ManagementOutcome =
  | {
      operationId: string;
      state: 'committed';
      result: { settings: RemoteSettings } | { device: ManagementDevice } | { removed: number };
      sessionEnded: boolean;
    }
  | { operationId: string; state: 'refused'; reason: RemoteRefusalCode }
  | { operationId: string; state: 'unknown'; reason: 'effect_outcome_unconfirmed' };
export type SettingChange =
  | { operationId: string; setting: 'open'; value: boolean }
  | { operationId: string; setting: 'sessions-per-device'; value: string | null };
export interface RemovalPreview {
  count: number;
  fingerprint: string;
}
export interface RemoteManagement {
  readonly clientId: string;
  previewRemoveOthers(): Promise<RemovalPreview>;
  removeOthers(input: { operationId: string; fingerprint: string }): Promise<ManagementOutcome>;
  settings(): Promise<SettingsView>;
  settings(options: { firestore: true }): Promise<FirestoreSettingsView>;
  devices(input: { cursor: string | null; limit: number }): Promise<DevicePage>;
  set(input: SettingChange): Promise<ManagementOutcome>;
  rename(input: {
    operationId: string;
    clientId: string;
    name: string;
  }): Promise<ManagementOutcome>;
  talk(input: {
    operationId: string;
    clientId: string;
    enabled: boolean;
  }): Promise<ManagementOutcome>;
  revoke(input: { operationId: string; clientId: string }): Promise<ManagementOutcome>;
  operation(operationId: string): Promise<ManagementOutcome>;
}
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const decimal = (value: unknown): value is string =>
  typeof value === 'string' && /^[1-9][0-9]{0,19}$/.test(value);
function settings(value: unknown): RemoteSettings {
  const row = object(value);
  exact(row, ['open', 'source', 'sessionsPerDevice', 'sessionsPerDeviceSource', 'warning']);
  valid(
    typeof row.open === 'boolean' && ['default', 'settings.json'].includes(row.source as string),
  );
  valid(row.sessionsPerDevice === null || decimal(row.sessionsPerDevice));
  valid(['default', 'settings.json'].includes(row.sessionsPerDeviceSource as string));
  valid(row.warning === null || row.warning === 'settings.json could not be read; defaults apply');
  return row as unknown as RemoteSettings;
}
function device(value: unknown, activity = false): ManagementDevice {
  const row = object(value);
  exact(row, [
    'clientId',
    'name',
    'kind',
    'issuedAtMs',
    'expiresAtMs',
    'revision',
    'revoked',
    'talkEnabled',
    ...(activity ? ['thisBrowser', 'liveSessionCount', 'lastActivityAtMs', 'lastUsedAtMs'] : []),
  ]);
  valid(typeof row.clientId === 'string' && UUID.test(row.clientId));
  valid(
    typeof row.name === 'string' &&
      new TextEncoder().encode(row.name).length <= 64 &&
      row.name.trim().length > 0 &&
      ![...row.name].some((char) => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127),
  );
  valid(['browser', 'addon', 'cli'].includes(row.kind as string) && integer(row.issuedAtMs));
  valid(row.expiresAtMs === null || integer(row.expiresAtMs));
  valid(
    integer(row.revision) &&
      (row.revision as number) > 0 &&
      typeof row.revoked === 'boolean' &&
      typeof row.talkEnabled === 'boolean',
  );
  if (activity)
    valid(
      typeof row.thisBrowser === 'boolean' &&
        integer(row.liveSessionCount) &&
        (row.lastActivityAtMs === null || integer(row.lastActivityAtMs)) &&
        (row.lastUsedAtMs === null || integer(row.lastUsedAtMs)),
    );
  return row as unknown as ManagementDevice;
}
const refusalCodes = new Set<string>([
  'REMOTE_INVITATION_BUSY',
  'REMOTE_INVITATION_CAPACITY',
  'REMOTE_INVITATION_UNAVAILABLE',
  'REMOTE_INVITATION_INVALID',
  'REMOTE_INVITATION_STOPPED',
  'REMOTE_INVITATION_DECIDED',
  'REMOTE_ALREADY_PAIRED',
  'REMOTE_SCOPE_DENIED',
  'REMOTE_INPUT_INVALID',
  'REMOTE_RATE_LIMITED',
  'REMOTE_INTENT_CONFLICT',
  'REMOTE_CLOSED',
  'REMOTE_SESSION_ENDED',
  'REMOTE_SESSION_EVICTED',
  'REMOTE_INPUT_TOO_LARGE',
  'REMOTE_STATE_UNAVAILABLE',
  'REMOTE_MANAGEMENT_READ_ONLY',
  'REMOTE_MANAGEMENT_UNAVAILABLE',
  'REMOTE_DEVICE_REVOKED',
  'REMOTE_DEVICE_NOT_FOUND',
  'REMOTE_SETTINGS_UNAVAILABLE',
  'REMOTE_MANAGEMENT_CAPACITY',
  'REMOTE_DEVICE_LIST_CHANGED',
  'REMOTE_DEVICE_REMOVAL_LIMIT',
  'REMOTE_MEMBER_OWNER_REQUIRED',
  'REMOTE_MEMBER_CONFLICT',
]);
export function parseManagementOutcome(value: unknown, operationId: string): ManagementOutcome {
  const row = object(value);
  valid(row.operationId === operationId);
  if (row.state === 'committed') {
    exact(row, ['operationId', 'state', 'result', 'sessionEnded']);
    valid(typeof row.sessionEnded === 'boolean');
    const result = object(row.result);
    valid(Object.keys(result).length === 1);
    if (Object.hasOwn(result, 'settings')) settings(result.settings);
    else if (Object.hasOwn(result, 'removed'))
      valid(
        integer(result.removed) && (result.removed as number) <= 1000 && row.sessionEnded === false,
      );
    else device(result.device);
  } else {
    exact(row, ['operationId', 'state', 'reason']);
    if (row.state === 'unknown') valid(row.reason === 'effect_outcome_unconfirmed');
    else
      valid(
        row.state === 'refused' && typeof row.reason === 'string' && refusalCodes.has(row.reason),
      );
  }
  return row as unknown as ManagementOutcome;
}
/** Constructing this helper opens nothing. Unknown mutations are never resent.
 * After Session loss, explicitly reopen once and read operation(originalId).
 * Failed fresh admission changes access status; it cannot establish mutation outcome.
 */
export function management(
  session: Session,
  options: { timeoutMs?: number } = {},
): RemoteManagement {
  const call = verifiedSessionRequest(session, options);
  const clientId = channelFor(session)!.paired.clientId;
  async function mutate(
    operation: string,
    value: {
      operationId: string;
      clientId?: string;
      name?: string;
      enabled?: boolean;
      fingerprint?: string;
    },
  ): Promise<ManagementOutcome> {
    const { operationId } = value;
    input(UUID.test(operationId));
    return managementMutation(call, operation, value, (reply) => {
      const parsed = parseManagementOutcome(reply, operationId);
      if (parsed.state === 'committed') {
        if (operation === 'remote.settings.set') valid('settings' in parsed.result);
        else if (operation === 'remote.devices.remove-others')
          valid('removed' in parsed.result && parsed.sessionEnded === false);
        else {
          valid('device' in parsed.result && parsed.result.device.clientId === value.clientId);
          if (operation === 'remote.devices.rename')
            valid(parsed.result.device.name === value.name);
          if (operation === 'remote.devices.revoke') valid(parsed.result.device.revoked);
          if (operation === 'remote.devices.talk')
            valid(parsed.result.device.talkEnabled === value.enabled);
        }
      }
      return parsed;
    });
  }

  return {
    clientId,
    previewRemoveOthers: () =>
      call('remote.devices.remove-others.preview', crypto.randomUUID(), {}, (value) => {
        const row = object(value);
        exact(row, ['count', 'fingerprint']);
        valid(
          integer(row.count) &&
            (row.count as number) <= 1000 &&
            typeof row.fingerprint === 'string' &&
            /^[0-9a-f]{64}$/.test(row.fingerprint),
        );
        return row as unknown as RemovalPreview;
      }),
    removeOthers: (change) => {
      input(typeof change.fingerprint === 'string' && /^[0-9a-f]{64}$/.test(change.fingerprint));
      return mutate('remote.devices.remove-others', { ...change });
    },
    settings: ((options?: { firestore: true }) =>
      call('remote.settings.show', crypto.randomUUID(), options ?? {}, (value) => {
        const row = object(value);
        exact(row, [
          'settings',
          'capabilities',
          'readOnlyReason',
          ...(options ? ['firestoreLayers', 'firestoreBudget'] : []),
        ]);
        if (options) {
          parseFirestoreLayers(row.firestoreLayers);
          parsePublishedLimits(row.firestoreBudget);
        }
        settings(row.settings);
        const capabilities = object(row.capabilities);
        exact(capabilities, ['settingsWrite', 'devicesWrite']);
        valid(
          typeof capabilities.settingsWrite === 'boolean' &&
            typeof capabilities.devicesWrite === 'boolean',
        );
        valid(row.readOnlyReason === null || row.readOnlyReason === 'local_cli_required');
        valid(
          capabilities.settingsWrite === capabilities.devicesWrite &&
            capabilities.settingsWrite === (row.readOnlyReason === null),
        );
        return row as unknown as SettingsView;
      })) as RemoteManagement['settings'],
    devices: (page) => {
      input(
        Number.isInteger(page.limit) &&
          page.limit >= 1 &&
          page.limit <= 50 &&
          (page.cursor === null ||
            (typeof page.cursor === 'string' && /^[A-Za-z0-9_-]{98}$/.test(page.cursor))),
      );
      return call('remote.devices.list', crypto.randomUUID(), page, (value) => {
        const row = object(value);
        exact(row, ['devices', 'nextCursor']);
        valid(Array.isArray(row.devices) && row.devices.length <= page.limit);
        const ids = new Set<string>();
        for (const entry of row.devices) {
          const item = device(entry, true);
          valid(!item.revoked && !ids.has(item.clientId));
          ids.add(item.clientId);
        }
        valid(
          row.nextCursor === null ||
            (typeof row.nextCursor === 'string' && /^[A-Za-z0-9_-]{98}$/.test(row.nextCursor)),
        );
        return row as unknown as DevicePage;
      });
    },
    set: (change) => {
      input(
        (change.setting === 'open' && typeof change.value === 'boolean') ||
          (change.setting === 'sessions-per-device' &&
            (change.value === null || decimal(change.value))),
      );
      return mutate('remote.settings.set', { ...change });
    },
    rename: (change) => {
      input(
        UUID.test(change.clientId) &&
          typeof change.name === 'string' &&
          change.name.trim().length > 0 &&
          new TextEncoder().encode(change.name).length <= 64 &&
          ![...change.name].some((char) => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127),
      );
      return mutate('remote.devices.rename', { ...change });
    },
    talk: (change) => {
      input(UUID.test(change.clientId) && typeof change.enabled === 'boolean');
      return mutate('remote.devices.talk', { ...change });
    },
    revoke: (change) => {
      input(UUID.test(change.clientId));
      return mutate('remote.devices.revoke', { ...change });
    },
    operation: (operationId) => {
      input(UUID.test(operationId));
      return call('remote.management.operation', crypto.randomUUID(), { operationId }, (value) =>
        parseManagementOutcome(value, operationId),
      );
    },
  };
}
