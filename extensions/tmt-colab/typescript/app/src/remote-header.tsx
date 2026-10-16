import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from 'react';
import {
  UiTheme,
  TooltipProvider,
  Tooltip,
  TooltipTrigger,
  TooltipContent,
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSub,
  DropdownMenuSubTrigger,
  DropdownMenuSubContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
} from '@tmt/browser-ui/ui';
import '@tmt/browser-ui/ui.css';
import { Ellipsis } from 'lucide-react';
import type { BrowserMenuItem } from '@tmt/browser-ui/react';
import type { RemotePresence, RemotePresenceSnapshot, RemoteDevice } from './remote-presence.js';
import { getTheme, subscribeTheme } from './theme.js';
import { useNotificationModalPresence } from './reply-notification-context.js';
import { text } from './strings.js';

const Context = createContext<RemotePresence | undefined>(undefined);
const hidden: RemotePresenceSnapshot = { kind: 'hidden' };
const absent = () => hidden;
const noSubscription = () => () => {};
export function RemotePresenceProvider({
  owner,
  children,
}: {
  owner?: RemotePresence;
  children: ReactNode;
}) {
  useEffect(() => {
    if (!owner) return;
    void owner.refresh();
    const visible = () => {
      if (document.visibilityState === 'visible') void owner.refresh();
    };
    document.addEventListener('visibilitychange', visible);
    return () => document.removeEventListener('visibilitychange', visible);
  }, [owner]);
  return <Context.Provider value={owner}>{children}</Context.Provider>;
}
function usePresence() {
  const owner = useContext(Context);
  const snapshot = useSyncExternalStore(
    owner?.subscribe ?? noSubscription,
    owner?.snapshot ?? absent,
    absent,
  );
  return {
    snapshot,
    refresh: () => {
      void owner?.refresh();
    },
  };
}
function label(snapshot: Exclude<RemotePresenceSnapshot, { kind: 'hidden' }>) {
  return snapshot.kind === 'confirmed' ? text.remoteCount(snapshot.count) : text.remoteUnconfirmed;
}
function accessible(snapshot: Exclude<RemotePresenceSnapshot, { kind: 'hidden' }>) {
  return snapshot.kind === 'confirmed'
    ? text.remoteCountLabel(snapshot.count)
    : text.remoteUnconfirmedLabel;
}
function activity(device: RemoteDevice, atMs: number) {
  if (device.lastActivityAtMs === null) return text.remoteActivityUnknown;
  const minutes = Math.floor(Math.max(0, atMs - device.lastActivityAtMs) / 60000);
  if (!minutes) return device.thisBrowser ? text.remoteActiveNow : text.remoteJustNow;
  if (minutes < 60) return text.remoteMinutesAgo(minutes);
  if (minutes < 1440) return text.remoteHoursAgo(Math.floor(minutes / 60));
  return text.remoteDaysAgo(Math.floor(minutes / 1440));
}
function details(snapshot: Exclude<RemotePresenceSnapshot, { kind: 'hidden' }>) {
  const rows =
    snapshot.kind === 'confirmed'
      ? snapshot.devices.map((device) =>
          text.remoteDeviceRow(
            device.thisBrowser
              ? text.remoteThisBrowser
              : device.kind === 'addon'
                ? text.remoteAddon
                : device.kind === 'cli'
                  ? text.remoteCli
                  : device.name,
            device.liveSessionCount,
            activity(device, snapshot.atMs),
          ),
        )
      : [text.remoteCouldNotConfirm];
  const time = new Date(snapshot.atMs).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  });
  return {
    rows,
    note:
      snapshot.kind === 'confirmed'
        ? text.remoteSnapshot(time)
        : text.remoteSnapshotUnconfirmed(time),
  };
}
function Details({ snapshot }: { snapshot: Exclude<RemotePresenceSnapshot, { kind: 'hidden' }> }) {
  const { rows, note } = details(snapshot);
  return (
    <div className="remote-details">
      {rows.map((row, index) => (
        <div key={index}>{row}</div>
      ))}
      <div className="remote-snapshot">{note}</div>
    </div>
  );
}
export function RemoteHeaderChip() {
  const { snapshot, refresh } = usePresence();
  const theme = useSyncExternalStore(subscribeTheme, getTheme, () => 'light' as const);
  if (snapshot.kind === 'hidden') return null;
  const muted = snapshot.kind === 'unconfirmed' || snapshot.count === 0;
  return (
    <span className="remote-header-chip" data-muted={muted}>
      <UiTheme theme={theme}>
        <TooltipProvider>
          <Tooltip
            onOpenChange={(open) => {
              if (open) refresh();
            }}
          >
            <TooltipTrigger asChild>
              {snapshot.kind === 'confirmed' ? (
                <a
                  className="tmt-ui-action remote-chip-action"
                  data-variant="text"
                  href={snapshot.settingsUrl}
                  aria-label={accessible(snapshot)}
                >
                  {label(snapshot)}
                </a>
              ) : (
                <button
                  type="button"
                  className="tmt-ui-action remote-chip-action"
                  data-variant="text"
                  aria-label={accessible(snapshot)}
                >
                  {label(snapshot)}
                </button>
              )}
            </TooltipTrigger>
            <TooltipContent>
              <Details snapshot={snapshot} />
            </TooltipContent>
          </Tooltip>
        </TooltipProvider>
      </UiTheme>
    </span>
  );
}

/** Narrow More reuses the existing shared submenu, without a shared header/menu API change. */
export function RemoteMoreMenu({
  items = [],
  onSelect = () => {},
}: {
  items?: readonly BrowserMenuItem[];
  onSelect?(key: string): void;
}) {
  const { snapshot, refresh } = usePresence();
  const theme = useSyncExternalStore(subscribeTheme, getTheme, () => 'light' as const);
  const report = useNotificationModalPresence();
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const pendingAction = useRef<string | undefined>(undefined);
  if (snapshot.kind === 'hidden' && !items.length) return null;
  const choices = items.filter((item) => item.checked !== undefined);
  return (
    <span className="remote-more-menu">
      <UiTheme theme={theme}>
        <DropdownMenu
          open={open}
          onOpenChange={(value) => {
            setOpen(value);
            report(value);
          }}
        >
          <DropdownMenuTrigger asChild>
            <button
              className="tmt-ui-action remote-chip-action remote-more-trigger"
              data-variant="text"
              type="button"
              ref={trigger}
              aria-label={text.more}
              onPointerDownCapture={(event) => {
                if (!event.isTrusted) {
                  event.preventDefault();
                  event.stopPropagation();
                }
              }}
              onKeyDownCapture={(event) => {
                if (!event.isTrusted || event.nativeEvent.isComposing) {
                  event.preventDefault();
                  event.stopPropagation();
                }
              }}
            >
              <Ellipsis aria-hidden="true" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="end"
            onCloseAutoFocus={(event) => {
              const action = pendingAction.current;
              if (action === undefined) return;
              pendingAction.current = undefined;
              event.preventDefault();
              // Native page panels capture their opener on mount. Complete the
              // shared menu's dismissal before invoking that existing owner.
              trigger.current?.focus({ preventScroll: true });
              onSelect(action);
            }}
            onClickCapture={(event) => {
              if (!event.isTrusted) {
                event.preventDefault();
                event.stopPropagation();
              }
            }}
            onKeyDownCapture={(event) => {
              if (!event.isTrusted || event.nativeEvent.isComposing) {
                event.preventDefault();
                event.stopPropagation();
              }
            }}
          >
            {snapshot.kind !== 'hidden' && (
              <DropdownMenuSub
                onOpenChange={(open) => {
                  if (open) refresh();
                }}
              >
                <DropdownMenuSubTrigger
                  aria-label={accessible(snapshot)}
                  className="remote-count"
                  data-muted={snapshot.kind === 'unconfirmed' || snapshot.count === 0}
                >
                  {label(snapshot)}
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent className="remote-submenu">
                  <DropdownMenuLabel>
                    <Details snapshot={snapshot} />
                  </DropdownMenuLabel>
                  {snapshot.kind === 'confirmed' && (
                    <DropdownMenuItem asChild>
                      <a href={snapshot.settingsUrl}>{text.remoteSettings}</a>
                    </DropdownMenuItem>
                  )}
                </DropdownMenuSubContent>
              </DropdownMenuSub>
            )}
            {items.map((item) =>
              item.checked === undefined ? (
                <DropdownMenuItem
                  key={item.key}
                  disabled={item.disabled}
                  onSelect={(event) => {
                    event.preventDefault();
                    pendingAction.current = item.key;
                    setOpen(false);
                    report(false);
                  }}
                >
                  {item.label}
                </DropdownMenuItem>
              ) : item === choices[0] ? (
                <DropdownMenuRadioGroup
                  key="theme"
                  value={choices.find((choice) => choice.checked)?.key}
                  onValueChange={onSelect}
                >
                  {choices.map((choice) => (
                    <DropdownMenuRadioItem
                      key={choice.key}
                      value={choice.key}
                      disabled={choice.disabled}
                    >
                      {choice.label}
                    </DropdownMenuRadioItem>
                  ))}
                </DropdownMenuRadioGroup>
              ) : null,
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      </UiTheme>
    </span>
  );
}
export function useRemoteHeaderAvailable(): boolean {
  return usePresence().snapshot.kind !== 'hidden';
}

/** The More owner stays mounted while its first snapshot is pending or unavailable. */
export function useRemoteHeaderSource(): boolean {
  return useContext(Context) !== undefined;
}
