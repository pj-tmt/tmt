import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { BrowserShell, BrowserTopBar, HostedShell, useTopBarReady } from '../../src/react.js';
import { mountShell, type ShellCommonControls } from '../../src/shell.js';
import { startShellHost, type ShellSlot, type ShellRoute } from '../../src/shell-host.js';
import '../../generated/static.css';

const common: ShellCommonControls = {
  openSearch() {},
  remote: { kind: 'unconfirmed', href: '/settings' },
};
const view = { rail: { main: [], bottom: [] } };
function scaffold() {
  const root = document.createElement('div');
  root.className = 'tmt-ui-shell';
  root.innerHTML =
    '<div class="tmt-ui-shell-content"><div class="tmt-ui-shell-bar"></div><div class="tmt-ui-shell-main"></div></div>';
  document.body.append(root);
  return root;
}
function refuses(adopt: () => unknown, reason: string) {
  try {
    adopt();
    return false;
  } catch (error) {
    return error instanceof Error && error.message.includes(reason);
  }
}

export function nativeReadiness() {
  const root = scaffold();
  const controls = mountShell(root, view);
  const states: boolean[] = [];
  try {
    states.push(controls.topBarReady());
    const missingRefused = refuses(controls.setTopBar, 'install common controls');
    controls.setCommonControls(common);
    states.push(controls.topBarReady());
    // A read does not adopt or change the legacy header.
    const barsBeforeAdoption = root.querySelectorAll('.tmt-ui-topbar').length;
    controls.setTopBar().actions.append(document.createTextNode('Native action'));
    const action = root.querySelector('.tmt-ui-topbar-actions')?.textContent;
    controls.clearTopBar();
    states.push(controls.topBarReady());
    controls.setCommonControls(undefined);
    states.push(controls.topBarReady());
    const clearedRefused = refuses(controls.setTopBar, 'install common controls');
    controls.setCommonControls(common);
    states.push(controls.topBarReady());
    controls.dispose();
    states.push(controls.topBarReady());
    controls.setCommonControls(common);
    states.push(controls.topBarReady());
    return {
      states,
      barsBeforeAdoption,
      action,
      missingRefused,
      clearedRefused,
      disposedRefused: refuses(controls.setTopBar, 'disposed'),
    };
  } finally {
    controls.dispose();
    root.remove();
  }
}

export async function hostedReadiness() {
  const root = scaffold();
  const original = location.href;
  const slots: ShellSlot[] = [];
  const mounts: Promise<void>[] = [];
  const disposalReadings: boolean[] = [];
  const routes: ShellRoute[] = ['first', 'second'].map((id) => {
    let mounted!: () => void;
    mounts.push(new Promise<void>((resolve) => (mounted = resolve)));
    return {
      id,
      paths: [`/ready-${id}/`],
      rail: 'colab',
      quiet: true,
      load: async () => ({
        mount(slot) {
          slots.push(slot);
          mounted();
          return () => {
            if (slot.basePath === '/ready-second/') disposalReadings.push(slot.topBarReady());
          };
        },
      }),
    };
  });
  history.replaceState(null, '', '/ready-first/');
  const host = startShellHost(root, routes);
  const react = createRoot(document.createElement('div'));
  const readings: boolean[] = [];
  function Probe() {
    readings.push(useTopBarReady());
    return null;
  }
  function read(slot: ShellSlot) {
    flushSync(() =>
      react.render(
        <HostedShell slot={slot} menu={undefined}>
          <Probe />
        </HostedShell>,
      ),
    );
  }
  try {
    await mounts[0];
    const first = slots[0]!;
    read(first);
    const before = first.topBarReady();
    host.setCommonControls(common);
    read(first);
    const installed = first.topBarReady();
    first.setTopBar().actions.textContent = 'First action';
    host.navigate('/ready-second/');
    await mounts[1];
    const second = slots[1]!;
    const stale = first.topBarReady();
    read(first);
    read(second);
    second.setTopBar().actions.textContent = 'Second action';
    const action = root.querySelector('.tmt-ui-topbar-actions')?.textContent;
    host.setCommonControls(undefined);
    const cleared = second.topBarReady();
    read(second);
    const clearedRefused = refuses(second.setTopBar, 'install common controls');
    host.setCommonControls(common);
    const menu = (label: string) => ({
      title: 'Test',
      groups: [
        { id: 'shortcuts', label, items: [], headerAction: { id: 'manage', label: 'Manage' } },
      ],
    });
    second.setMenu(menu('Current'));
    first.setMenu(menu('Stale'));
    const menuAfterStale = root.querySelector('.tmt-ui-rail-sub')?.textContent;
    host.dispose();
    second.setMenu(menu('Disposed'));
    const menuAfterDispose = root.querySelector('.tmt-ui-rail-sub')?.textContent ?? null;
    const disposed = second.topBarReady();
    read(second);
    return {
      before,
      installed,
      stale,
      cleared,
      disposed,
      readings,
      disposalReadings,
      action,
      clearedRefused,
      menuAfterStale,
      menuAfterDispose,
    };
  } finally {
    flushSync(() => react.unmount());
    host.dispose();
    root.remove();
    history.replaceState(null, '', original);
  }
}

export function reactReadiness() {
  const container = document.createElement('div');
  document.body.append(container);
  const root = createRoot(container);
  const readings: boolean[] = [];
  function Probe() {
    const ready = useTopBarReady();
    readings.push(ready);
    return ready ? <BrowserTopBar actions={<button>React action</button>} /> : <p>Legacy header</p>;
  }
  try {
    flushSync(() => root.render(<Probe />));
    const outside = readings.at(-1);
    flushSync(() =>
      root.render(
        <BrowserShell key="legacy" view={view}>
          <Probe />
        </BrowserShell>,
      ),
    );
    const absent = readings.at(-1);
    const legacy = container.querySelector('p')?.textContent;
    // The root installs common controls before mounting its replacement route.
    flushSync(() =>
      root.render(
        <BrowserShell key="ready" view={view} commonControls={common}>
          <Probe />
        </BrowserShell>,
      ),
    );
    const installed = readings.at(-1);
    const action = container.querySelector('.tmt-ui-topbar-actions button')?.textContent;
    flushSync(() =>
      root.render(
        <BrowserShell key="cleared" view={view}>
          <Probe />
        </BrowserShell>,
      ),
    );
    const cleared = readings.at(-1);
    const barsAfterClear = container.querySelectorAll('.tmt-ui-topbar').length;
    return { outside, absent, legacy, installed, action, cleared, barsAfterClear };
  } finally {
    flushSync(() => root.unmount());
    container.remove();
  }
}
