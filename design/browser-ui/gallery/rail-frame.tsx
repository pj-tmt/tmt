import { useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { BrowserShell, BrowserTopBar } from '../src/react.js';
import {
  mountShell,
  shellCount,
  type ShellControls,
  type ShellView,
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
  const [selected, setSelected] = useState('all');
  const [revision, setRevision] = useState(0);
  const [status, setStatus] = useState('Ready');
  const [mounted, setMounted] = useState(true);
  const view = useMemo<ShellView>(
    () => ({
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
              label: 'Manage projects',
              icon: 'settings',
              onActivate: () => setStatus(`Manage ${revision}`),
            },
            items: [
              {
                id: 'project',
                label: 'Release planning and coordination',
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
            add: { label: '+ New project', onActivate: () => setStatus(`New ${revision}`) },
          },
        ],
        settings: {
          id: 'colab-settings',
          label: 'Colab settings',
          href: '/colab/settings',
          current: selected === 'colab-settings',
        },
      },
    }),
    [selected, revision],
  );
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
      <p role="status">{status}</p>
      <button onClick={() => setRevision((n) => n + 1)}>Refresh command bindings</button>
      <button onClick={() => setSelected('module')}>Select module</button>
      <button onClick={() => setMounted(false)}>Unmount rail</button>
    </div>
  );
  if (!mounted) return <p>Rail unmounted</p>;
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
