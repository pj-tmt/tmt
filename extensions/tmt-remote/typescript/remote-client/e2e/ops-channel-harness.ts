/** Test-owned source; real Ops route, shared shell and read lifecycle. No native/Core transport. */
import { mountShell, shellRail } from '@tmt/browser-ui/shell';
import { applyStoredTheme } from '@tmt/browser-ui/theme';
import { setRouteTitle, type RemoteShell } from '../src/remote-shell.js';
import { opsPage } from '../src/ops-page.js';
import type { OpsBoard } from '../src/ops-source.js';

declare global {
  interface Window {
    channelBoard: OpsBoard;
    channelReads: number;
    channelStop(): void;
    channelHold: boolean;
    channelRelease(): void;
  }
}
window.channelReads = 0;
window.channelHold = false;
window.channelBoard = {
  squads: [{ name: 'sample', lead: 'sample-lead' }],
  agents: [
    {
      id: '00000000-0000-4000-8000-000000000001',
      name: 'sample-lead',
      squad: 'sample',
      role: 'lead',
      state: 'working',
      presence: 'active',
      activity: { state: 'working' },
      waitingOnYou: 3,
      task: 'Review the sample plan',
    },
    {
      id: '00000000-0000-4000-8000-000000000002',
      name: 'sample-1',
      squad: 'sample',
      role: 'member',
      state: 'needs-you',
      presence: 'active',
      activity: { state: 'idle' },
      waitingOnYou: 4,
    },
    {
      id: '00000000-0000-4000-8000-000000000003',
      name: 'sample-2',
      squad: 'sample',
      role: 'member',
      state: 'offline',
      presence: 'offline',
      waitingOnYou: 0,
    },
  ],
  schedules: [
    {
      id: 'timer1',
      squad: 'sample',
      when: 'cron 0 * * * *',
      what: 'Sample hourly review',
      owner: 'sample-lead',
    },
  ],
  truncated: false,
};
applyStoredTheme();
const controller = new AbortController();
window.channelStop = () => controller.abort();
const rail = shellRail(undefined, { origin: location.origin, href: `${location.origin}/ops/` });
// This source is injected; the shell description names the real Ops module for rail ownership.
rail.main.find((item) => item.id === 'ops')!.disabled = false;
rail.main.find((item) => item.id === 'ops')!.current = true;
const shell = mountShell(document.querySelector('.tmt-ui-shell')!, { rail });
shell.setCommonControls({ openSearch() {}, remote: { kind: 'unconfirmed', href: '/remote/' } });
opsPage(
  {
    signal: controller.signal,
    setMenu: (menu) => shell.update({ rail, menu }),
    setTitle: (title, ancestors) =>
      setRouteTitle(
        {
          ...shell,
          setTitle: (text) => {
            document.title = `${text} · TMT`;
          },
        },
        title,
        ancestors,
      ),
  } as RemoteShell,
  {
    async board() {
      window.channelReads++;
      const board = structuredClone(window.channelBoard);
      if (window.channelHold)
        await new Promise<void>((resolve) => {
          window.channelRelease = resolve;
        });
      return board;
    },
  },
);
