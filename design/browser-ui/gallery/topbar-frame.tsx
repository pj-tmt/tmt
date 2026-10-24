import { useCallback, useLayoutEffect, useMemo, useRef, useState, type RefObject } from 'react';
import {
  BrowserAction,
  BrowserDialog,
  BrowserField,
  BrowserShell,
  BrowserTopBar,
} from '../src/react.js';
import {
  mountShell,
  type ShellCommonControls,
  type ShellControls,
  type ShellView,
} from '../src/shell.js';
import { startShellHost, type ShellHost, type ShellRoute } from '../src/shell-host.js';

const destinations = [
  ['home', 'Home'],
  ['colab', 'Colab pages'],
  ['projects', 'Colab projects'],
  ['remote', 'Remote'],
  ['devices', 'Remote devices'],
  ['remote-settings', 'Remote settings'],
  ['agents', 'Ops agents'],
  ['squads', 'Ops squads'],
  ['timers', 'Ops timers'],
  ['ops-settings', 'Ops settings'],
  ['digest', 'Digest'],
  ['settings', 'TMT settings'],
] as const;
const view: ShellView = {
  rail: {
    main: [
      { id: 'home', label: 'Home', icon: 'house', href: '/', current: true },
      { id: 'colab', label: 'Colab', icon: 'file-text', href: '/colab/', current: false },
      {
        id: 'remote',
        label: 'Remote',
        icon: 'monitor-smartphone',
        href: '/remote/',
        current: false,
      },
      { id: 'ops', label: 'Ops', icon: 'users', href: '/ops/', current: false },
      { id: 'digest', label: 'Digest', icon: 'chart-column', href: '/digest/', current: false },
    ],
    bottom: [
      {
        id: 'settings',
        label: 'TMT settings',
        icon: 'settings',
        href: '/settings/',
        current: false,
      },
    ],
  },
};

function NativeTopBar({
  common,
  hosted,
  activate,
  owner,
  gate,
  didMount,
}: {
  common: ShellCommonControls;
  hosted: boolean;
  activate(): void;
  owner: RefObject<ShellControls | ShellHost | null>;
  gate?: Promise<void>;
  didMount(): void;
}) {
  const root = useRef<HTMLDivElement>(null);
  const latest = useRef({ common, activate, didMount });
  latest.current = { common, activate, didMount };
  useLayoutEffect(() => {
    const node = root.current!;
    const main = node.querySelector<HTMLElement>('.tmt-ui-shell-main')!;
    const fill = (actions: HTMLElement, title: string) => {
      latest.current.didMount();
      const heading = document.createElement('h1');
      heading.className = 'tmt-ui-page-title';
      heading.textContent = title;
      heading.tabIndex = -1;
      const note = document.createElement('p');
      note.textContent =
        'Gallery root bindings only. Product search remains in its owning adoption.';
      main.replaceChildren(heading, note);
      heading.focus();
      const action = document.createElement('button');
      action.type = 'button';
      action.className = 'tmt-ui-action';
      action.dataset.variant = 'secondary';
      action.textContent = 'Module action';
      action.onclick = (event) => {
        if (event.isTrusted) latest.current.activate();
      };
      actions.append(action);
    };
    if (hosted) {
      const route = (id: string, label: string, path: string): ShellRoute => ({
        id,
        rail: id,
        paths: [path],
        quiet: true,
        load: async () => {
          await gate;
          return {
            mount(slot) {
              fill(slot.setTopBar().actions, label);
            },
          };
        },
      });
      const routes = destinations.map(([id, label]) => route(id, label, `/topbar-${id}/`));
      routes.push(route('home', 'Shared top bar', '/'));
      const host = startShellHost(node, routes);
      host.setCommonControls(latest.current.common);
      owner.current = host;
    } else {
      const controls = mountShell(node, view, () => true);
      controls.setCommonControls(latest.current.common);
      fill(controls.setTopBar().actions, 'Shared top bar');
      owner.current = controls;
    }
    return () => {
      owner.current?.dispose();
      owner.current = null;
    };
  }, [hosted, owner, gate]);
  useLayoutEffect(() => owner.current?.setCommonControls(common), [common, owner]);
  // Only the scaffold belongs to React. Its contents belong to the actual native host.
  return (
    <div ref={root} className="tmt-ui-shell" data-native-topbar>
      <div className="tmt-ui-shell-content">
        <div className="tmt-ui-shell-bar" />
        <div className="tmt-ui-shell-main tmt-ui-page" />
      </div>
    </div>
  );
}

function deferredLoad() {
  let release!: () => void;
  const promise = new Promise<void>((resolve) => {
    release = resolve;
  });
  return { promise, release };
}

/** The real public root bindings, renderer and adapter; the search callback is a gallery fixture. */
export function TopBarFrame({ params }: { params: URLSearchParams }) {
  const renderer = params.get('renderer') ?? 'react';
  const [status, setStatus] = useState<ShellCommonControls['remote']>({
    kind: 'confirmed',
    liveSessions: 6,
    href: '/settings',
  });
  const [search, setSearch] = useState(false);
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState('none');
  const [actions, setActions] = useState(0);
  const [shown, setShown] = useState(true);
  const [restoreSearchFocus, setRestoreSearchFocus] = useState(true);
  const [searchRequests, setSearchRequests] = useState(0);
  const [moduleMounts, setModuleMounts] = useState(0);
  const delayed = params.get('delay') === 'true';
  const gate = useMemo(() => (delayed ? deferredLoad() : undefined), [delayed]);
  const native = useRef<ShellControls | ShellHost | null>(null);
  const opener = useRef<HTMLButtonElement | null>(null);
  const openSearch = useCallback((control: HTMLButtonElement) => {
    opener.current = control;
    control.focus();
    setRestoreSearchFocus(true);
    setSearchRequests((value) => value + 1);
    setSearch(true);
  }, []);
  useLayoutEffect(() => {
    const status = (event: Event) => {
      const value = (event as CustomEvent<string>).detail;
      setStatus(
        value === 'unknown'
          ? { kind: 'unconfirmed', href: '/settings' }
          : {
              kind: 'confirmed',
              liveSessions: value === 'zero' ? 0 : 6,
              href: '/settings',
            },
      );
    };
    const hide = () => setShown(false);
    const release = () => gate?.release();
    window.addEventListener('topbar-fixture-status', status);
    window.addEventListener('topbar-fixture-hide', hide);
    window.addEventListener('topbar-fixture-release', release);
    return () => {
      window.removeEventListener('topbar-fixture-status', status);
      window.removeEventListener('topbar-fixture-hide', hide);
      window.removeEventListener('topbar-fixture-release', release);
    };
  }, [gate]);
  const common = useMemo(() => ({ openSearch, remote: status }), [openSearch, status]);
  const activate = () => setActions((n) => n + 1);
  return (
    <>
      {shown &&
        (renderer === 'react' ? (
          <BrowserShell view={view} commonControls={common} select={() => true}>
            <BrowserTopBar
              actions={
                <BrowserAction
                  type="button"
                  variant="secondary"
                  label="Module action"
                  onActivate={activate}
                />
              }
            />
            <div className="tmt-ui-page">
              <h1 className="tmt-ui-page-title">Shared top bar</h1>
              <p>Gallery root bindings only. Product search remains in its owning adoption.</p>
            </div>
          </BrowserShell>
        ) : (
          <NativeTopBar
            common={common}
            hosted={renderer === 'hosted'}
            activate={activate}
            owner={native}
            gate={gate?.promise}
            didMount={() => setModuleMounts((value) => value + 1)}
          />
        ))}
      <div className="gallery-topbar-fixture-controls">
        <BrowserAction
          type="button"
          variant="secondary"
          label="Six sessions"
          onActivate={() => setStatus({ kind: 'confirmed', liveSessions: 6, href: '/settings' })}
        />
        <BrowserAction
          type="button"
          variant="secondary"
          label="No sessions"
          onActivate={() => setStatus({ kind: 'confirmed', liveSessions: 0, href: '/settings' })}
        />
        <BrowserAction
          type="button"
          variant="secondary"
          label="Unconfirmed sessions"
          onActivate={() => setStatus({ kind: 'unconfirmed', href: '/settings' })}
        />
        <BrowserAction
          type="button"
          variant="secondary"
          label="Toggle shell"
          onActivate={() => setShown((value) => !value)}
        />
        <output data-testid="module-actions">{actions}</output>
        <output data-testid="search-selection">{selected}</output>
        <output data-testid="search-requests">{searchRequests}</output>
        <output data-testid="module-mounts">{moduleMounts}</output>
      </div>
      <BrowserDialog
        open={search}
        title="Search TMT"
        onDismiss={() => setSearch(false)}
        focusFallback={() => opener.current}
        restoreFocus={restoreSearchFocus}
      >
        <p>
          Root opener demonstration, not a second product search. These fixture navigation entries
          need no data reads.
        </p>
        <BrowserField
          controlId="topbar-demo-query"
          label="Search"
          renderControl={(props) => (
            <input {...props} value={query} onChange={(event) => setQuery(event.target.value)} />
          )}
        />
        {destinations
          .filter(([, label]) => label.toLowerCase().includes(query.toLowerCase()))
          .map(([id, label]) => (
            <BrowserAction
              key={id}
              type="button"
              variant="secondary"
              label={label}
              onActivate={() => {
                setSelected(id);
                setSearch(false);
                if (native.current && 'navigate' in native.current) {
                  setRestoreSearchFocus(false);
                  native.current.navigate(`/topbar-${id}/?frame=topbar&renderer=hosted`);
                }
              }}
            />
          ))}
      </BrowserDialog>
    </>
  );
}
