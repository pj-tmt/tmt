import {
  createContext,
  useContext,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { mountShell, type ShellControls, type ShellMenu, type ShellView } from './shell.js';
import type { ShellSlot } from './shell-host.js';

export interface BrowserShellProps {
  view: ShellView;
  /** Take over a plain click on a rail or menu item; return true when handled. */
  select?(id: string, href: string): boolean;
  children: ReactNode;
}

const ShellBar = createContext<HTMLElement | null>(null);
/** The shell's header slot: a host renders its one header there (a portal), or in place when
 * there is no shell. */
export const useShellBar = () => useContext(ShellBar);

/** React hosts inside the one TMT web shell; the rail and menu use the shared static renderer. */
export function BrowserShell({ view, select, children }: BrowserShellProps) {
  const root = useRef<HTMLDivElement>(null);
  const controls = useRef<ShellControls | null>(null);
  const [bar, setBar] = useState<HTMLDivElement | null>(null);
  const latest = useRef(select);
  latest.current = select;
  useLayoutEffect(() => {
    controls.current = mountShell(root.current!, view, (id, href) =>
      Boolean(latest.current?.(id, href)),
    );
    return () => {
      controls.current?.dispose();
      controls.current = null;
    };
    // The renderer mounts once; later views go through update below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useLayoutEffect(() => {
    controls.current?.update(view);
  }, [view]);
  return (
    <div ref={root} className="tmt-ui-shell">
      <div className="tmt-ui-shell-content">
        <div ref={setBar} className="tmt-ui-shell-bar" />
        <div className="tmt-ui-shell-main">
          {/* Children mount once the header slot exists (the same commit, before paint), so a
              host header portals into it from its first mount and is never remounted. */}
          <ShellBar value={bar}>{bar ? children : null}</ShellBar>
        </div>
      </div>
    </div>
  );
}

export interface HostedShellProps {
  slot: ShellSlot;
  menu: ShellMenu | undefined;
  select?(id: string, href: string): boolean;
  children: ReactNode;
}

/** A React module inside the one TMT app: the host owns the rail; the module's header portals
 * into the host's slot and its menu goes to the host. */
export function HostedShell({ slot, menu, select, children }: HostedShellProps) {
  const latest = useRef(select);
  latest.current = select;
  useLayoutEffect(() => {
    slot.setMenu(menu, (id, href) => Boolean(latest.current?.(id, href)));
  }, [slot, menu]);
  return <ShellBar value={slot.bar}>{children}</ShellBar>;
}
