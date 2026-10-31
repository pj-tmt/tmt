import { useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { BrowserShell, BrowserTopBar } from '../src/react.js';
import {
  mountShell,
  shellCount,
  type ShellControls,
  type ShellView,
  type ShellMenuItem,
  type ShellCommonControls,
} from '../src/shell.js';

const common: ShellCommonControls = {
  openSearch() {},
  remote: { kind: 'unconfirmed', href: '/remote/' },
};
function NativeRail({
  view,
  select,
  children,
}: {
  view: ShellView;
  select(id: string): boolean;
  children: ReactNode;
}) {
  const root = useRef<HTMLDivElement>(null);
  const controls = useRef<ShellControls | null>(null);
  const latest = useRef(select);
  latest.current = select;
  useLayoutEffect(() => {
    const shell = mountShell(root.current!, view, (id) => latest.current(id));
    controls.current = shell;
    shell.setCommonControls(common);
    const heading = document.createElement('h1');
    heading.textContent = 'Shared rail';
    shell.setTopBar().title.append(heading);
    return () => {
      shell.dispose();
      controls.current = null;
    };
    // Mount once; updates share the same native renderer as BrowserShell.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useLayoutEffect(() => {
    controls.current?.update(view);
  }, [view]);
  return (
    <div ref={root} className="tmt-ui-shell">
      <div className="tmt-ui-shell-content">
        <div className="tmt-ui-shell-bar" />
        <main className="tmt-ui-shell-main">{children}</main>
      </div>
    </div>
  );
}

/** Sample domain data; only the shared rail is under review, not a product menu adoption. */
export function RailFrame({ params }: { params: URLSearchParams }) {
  const interactive = params.get('interactive') === 'true';
  const tree = params.get('tree') === 'true';
  const icons = params.get('icons') === 'true';
  const [selected, setSelected] = useState(interactive ? 'project' : tree ? 'night-1' : 'all');
  const [open, setOpen] = useState(['tmt-gui']);
  const [order, setOrder] = useState(['project', 'empty', 'notes']);
  const [holdMoves, setHoldMoves] = useState(false);
  const [reorderEnabled, setReorderEnabled] = useState(true);
  const [actionCalls, setActionCalls] = useState(0);
  const [reorderRequests, setReorderRequests] = useState(0);
  const [revision, setRevision] = useState(0);
  const [status, setStatus] = useState('Ready');
  const [mounted, setMounted] = useState(true);
  const view = useMemo<ShellView>(() => {
    const result: ShellView = {
      rail: {
        main: [
          {
            id: 'home',
            label: 'Home',
            href: '/',
            icon: 'house',
            current: false,
            count: shellCount(0),
          },
          {
            id: 'colab',
            label: 'Colab',
            href: '/colab/',
            icon: 'file-text',
            current: true,
            count: shellCount(3),
          },
          {
            id: 'remote',
            label: 'Remote',
            href: '/remote/',
            icon: 'monitor-smartphone',
            current: false,
            count: shellCount(9, 'plain'),
          },
          { id: 'ops', label: 'Ops', href: '/ops/', icon: 'users', current: false },
          { id: 'digest', label: 'Digest', href: '/digest/', icon: 'chart-column', current: false },
        ],
        bottom: [
          {
            id: 'settings',
            label: 'TMT settings',
            href: '/settings/',
            icon: 'settings',
            current: false,
          },
        ],
      },
      menu: {
        title: 'Colab sample pages',
        groups: [
          {
            items: [
              {
                id: 'all',
                label: 'All pages',
                ...(icons ? { icon: 'list' as const } : {}),
                href: '/colab/',
                current: selected === 'all',
                total: 42,
                count: shellCount(3),
              },
            ],
          },
          {
            label: 'Projects',
            action: {
              label: icons ? 'Manage' : 'Manage projects',
              ...(icons ? {} : { icon: 'settings' as const }),
              onActivate: () => setStatus(`Manage ${revision}`),
            },
            items: [
              {
                id: 'project',
                label: 'Release planning and coordination',
                ...(icons ? { icon: 'bookmark' as const } : {}),
                href: '/colab/project',
                current: selected === 'project',
                total: 6,
                count: shellCount(1),
              },
              {
                id: 'empty',
                label: 'Empty project',
                href: '/colab/empty',
                current: selected === 'empty',
                total: 0,
                count: shellCount(0),
              },
            ],
            add: { label: 'New project', onActivate: () => setStatus(`New ${revision}`) },
          },
        ],
        settings: {
          id: 'colab-settings',
          label: 'Colab settings',
          href: '/colab/settings',
          current: selected === 'colab-settings',
        },
      },
    };
    if (tree) {
      for (const item of result.rail.main) item.current = item.id === 'ops';
      const entry = (id: string, label: string, extra: Partial<ShellMenuItem> = {}) => ({
        id,
        label,
        href: `/ops/${id}`,
        current: selected === id,
        ...extra,
      });
      const squad = (id: string, members: ShellMenuItem[], extra: Partial<ShellMenuItem> = {}) =>
        entry(id, id, {
          children: members,
          expanded: open.includes(id),
          toggle: () =>
            setOpen((before) =>
              before.includes(id) ? before.filter((name) => name !== id) : [...before, id],
            ),
          ...extra,
        });
      result.menu = {
        title: 'Ops',
        groups: [
          {
            label: 'This machine',
            items: [
              squad(
                'tmt-gui',
                [
                  entry('tmt-gui-lead', 'tmt-gui-lead'),
                  entry('gui-2', 'gui-2', { emphasis: 'unread', count: shellCount(2, 'plain') }),
                  entry('gui-3', 'gui-3'),
                ],
                { emphasis: 'unread', count: shellCount(2, 'plain') },
              ),
              squad('night', [entry('night-lead', 'night-lead'), entry('night-1', 'night-1')]),
              entry('tmt-remote', 'tmt-remote', {
                emphasis: 'unread',
                count: shellCount(1, 'plain'),
              }),
            ],
          },
        ],
        settings: entry('ops-settings', 'Ops settings'),
      };
    }
    if (interactive) {
      const group = result.menu!.groups[1]!;
      group.label = 'Shortcuts';
      const entries = new Map(group.items.map((item) => [item.id, item]));
      entries.get('project')!.label = 'Colab v1 · In progress';
      entries.get('empty')!.label = 'Remote';
      entries.get('empty')!.total = 4;
      entries.set('notes', {
        id: 'notes',
        label: 'Needs my sign-off',
        href: '/colab/signoff',
        current: selected === 'notes',
        total: 2,
      });
      group.items = order.map((id): ShellMenuItem => {
        const item = entries.get(id)!;
        return {
          ...item,
          actions: [
            {
              label: `Edit ${item.label}`,
              icon: 'pencil',
              onActivate: () => {
                setActionCalls((n) => n + 1);
                setStatus(`Edit ${id} ${revision}`);
              },
            },
            {
              label: `Delete ${item.label}`,
              icon: 'trash-2',
              onActivate: () => {
                setActionCalls((n) => n + 1);
                setStatus(`Delete ${id} ${revision}`);
              },
            },
          ],
        };
      });
      group.action = {
        label: 'Manage shortcuts',
        icon: 'settings',
        onActivate: () => setStatus(`Manage ${revision}`),
      };
      group.add = { label: 'New shortcut', onActivate: () => setStatus(`New ${revision}`) };
      group.reorder = reorderEnabled
        ? (from, to) => {
            setReorderRequests((n) => n + 1);
            setStatus(`Reorder ${from} to ${to} ${revision}`);
            if (!holdMoves)
              setOrder((before) => {
                const moved = [...before];
                const fromIndex = moved.indexOf(from),
                  toIndex = moved.indexOf(to);
                if (fromIndex < 0 || toIndex < 0) return before;
                moved.splice(fromIndex, 1);
                moved.splice(toIndex, 0, from);
                return moved;
              });
          }
        : undefined;
    }
    return result;
  }, [selected, revision, interactive, tree, icons, open, order, holdMoves, reorderEnabled]);
  const select = (id: string) => {
    setSelected(id);
    return true;
  };
  const content = (
    <div className="tmt-ui-page">
      <p>
        Shared rail with sample counts and host-owned commands. Main area is a fixture, not a
        product adoption.
      </p>
      <p role="status" data-host-status>
        {status}
      </p>
      {tree && <p data-host-open={open.join(',')}>Host expanded: {open.join(', ') || 'none'}</p>}
      {interactive && (
        <>
          <p
            data-host-order={order.join(',')}
            data-action-calls={actionCalls}
            data-reorder-requests={reorderRequests}
          >
            Host order: {order.join(', ')}
          </p>
          <button onClick={() => setOrder(['project', 'empty', 'notes'])}>Reset host order</button>
          <button onClick={() => setHoldMoves((value) => !value)}>
            Toggle held reorder requests
          </button>
          <button onClick={() => setReorderEnabled((value) => !value)}>
            Toggle reorder availability
          </button>
        </>
      )}
      <button onClick={() => setRevision((n) => n + 1)}>Refresh command bindings</button>
      <button onClick={() => setSelected('module')}>Select module</button>
      <button onClick={() => setMounted(false)}>Unmount rail</button>
    </div>
  );
  if (!mounted)
    return (
      <p data-action-calls={actionCalls} data-reorder-requests={reorderRequests}>
        Rail unmounted
      </p>
    );
  return params.get('renderer') === 'native' ? (
    <NativeRail view={view} select={select}>
      {content}
    </NativeRail>
  ) : (
    <BrowserShell view={view} select={select} commonControls={common}>
      <BrowserTopBar title={<h1>Shared rail</h1>} />
      {content}
    </BrowserShell>
  );
}
