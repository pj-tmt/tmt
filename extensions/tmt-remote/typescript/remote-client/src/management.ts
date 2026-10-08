import type { Session } from './device.js';
import {
  ClientError,
  RefusalError,
  verifiedSessionRequest,
  type RemoteRefusalCode,
} from './operations.js';

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
}
export interface DevicePage {
  devices: (ManagementDevice & {
    thisBrowser: boolean;
    liveSessionCount: number;
    lastActivityAtMs: number | null;
  })[];
  nextCursor: string | null;
}
export interface SettingsView {
  settings: RemoteSettings;
  capabilities: { settingsWrite: boolean; devicesWrite: boolean };
  readOnlyReason: 'local_cli_required' | null;
}
export type ManagementOutcome =
  | {
      operationId: string;
      state: 'committed';
      result: { settings: RemoteSettings } | { device: ManagementDevice };
      sessionEnded: boolean;
    }
  | { operationId: string; state: 'refused'; reason: RemoteRefusalCode }
  | { operationId: string; state: 'unknown'; reason: 'effect_outcome_unconfirmed' };
export type SettingChange =
  | { operationId: string; setting: 'open'; value: boolean }
  | { operationId: string; setting: 'sessions-per-device'; value: string | null };
export interface RemoteManagement {
  settings(): Promise<SettingsView>;
  devices(input: { cursor: string | null; limit: number }): Promise<DevicePage>;
  set(input: SettingChange): Promise<ManagementOutcome>;
  rename(input: {
    operationId: string;
    clientId: string;
    name: string;
  }): Promise<ManagementOutcome>;
  revoke(input: { operationId: string; clientId: string }): Promise<ManagementOutcome>;
  operation(operationId: string): Promise<ManagementOutcome>;
}
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const decimal = (value: unknown): value is string =>
  typeof value === 'string' && /^[1-9][0-9]{0,19}$/.test(value);
function valid(condition: unknown): asserts condition {
  if (!condition) throw new Error('Invalid management response.');
}
function input(condition: unknown): asserts condition {
  if (!condition) throw new TypeError('Invalid management input.');
}
function object(value: unknown): Record<string, unknown> {
  valid(typeof value === 'object' && value !== null && !Array.isArray(value));
  return value as Record<string, unknown>;
}
function exact(value: Record<string, unknown>, keys: string[]): void {
  valid(
    Object.keys(value).length === keys.length && keys.every((key) => Object.hasOwn(value, key)),
  );
}
const integer = (value: unknown) => Number.isSafeInteger(value) && (value as number) >= 0;
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
    ...(activity ? ['thisBrowser', 'liveSessionCount', 'lastActivityAtMs'] : []),
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
  valid(integer(row.revision) && (row.revision as number) > 0 && typeof row.revoked === 'boolean');
  if (activity)
    valid(
      typeof row.thisBrowser === 'boolean' &&
        integer(row.liveSessionCount) &&
        (row.lastActivityAtMs === null || integer(row.lastActivityAtMs)),
    );
  return row as unknown as ManagementDevice;
}
const refusalCodes = new Set<string>([
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
]);
function outcome(value: unknown, operationId: string): ManagementOutcome {
  const row = object(value);
  valid(row.operationId === operationId);
  if (row.state === 'committed') {
    exact(row, ['operationId', 'state', 'result', 'sessionEnded']);
    valid(typeof row.sessionEnded === 'boolean');
    const result = object(row.result);
    valid(Object.keys(result).length === 1);
    if (Object.hasOwn(result, 'settings')) settings(result.settings);
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
  async function mutate(
    operation: string,
    value: { operationId: string; clientId?: string; name?: string },
  ): Promise<ManagementOutcome> {
    const { operationId } = value;
    input(UUID.test(operationId));
    try {
      return await call(
        operation,
        operationId,
        value,
        (reply) => {
          const parsed = outcome(reply, operationId);
          if (parsed.state === 'committed') {
            if (operation === 'remote.settings.set') valid('settings' in parsed.result);
            else {
              valid('device' in parsed.result && parsed.result.device.clientId === value.clientId);
              if (operation === 'remote.devices.rename')
                valid(parsed.result.device.name === value.name);
              if (operation === 'remote.devices.revoke') valid(parsed.result.device.revoked);
            }
          }
          return parsed;
        },
        true,
      );
    } catch (error) {
      if (error instanceof ClientError)
        throw new ClientError(error.code, error.message, operationId);
      if (error instanceof RefusalError)
        return { state: 'refused', operationId, reason: error.code };
      throw error;
    }
  }
  return {
    settings: () =>
      call('remote.settings.show', crypto.randomUUID(), {}, (value) => {
        const row = object(value);
        exact(row, ['settings', 'capabilities', 'readOnlyReason']);
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
      }),
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
          valid(!ids.has(item.clientId));
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
    revoke: (change) => {
      input(UUID.test(change.clientId));
      return mutate('remote.devices.revoke', { ...change });
    },
    operation: (operationId) => {
      input(UUID.test(operationId));
      return call('remote.management.operation', crypto.randomUUID(), { operationId }, (value) =>
        outcome(value, operationId),
      );
    },
  };
}
