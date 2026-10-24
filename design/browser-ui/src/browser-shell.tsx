import {
  createContext,
  useContext,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import {
  mountShell,
  type ShellControls,
  type ShellMenu,
  type ShellTools,
  type ShellView,
  type ShellCommonControls,
  type ShellTopBar,
} from './shell.js';
import { createPortal } from 'react-dom';
import type { ShellSlot } from './shell-host.js';

export interface BrowserShellProps {
  view: ShellView;
  /** Take over a plain click on a rail or menu item; return true when handled. */
  select?(id: string, href: string): boolean;
  children: ReactNode;
  /** Root-owned Search opener and admitted Remote display projection. */
  commonControls?: ShellCommonControls;
}

const TopBar = createContext<(() => ShellTopBar) | null>(null);

export interface BrowserTopBarProps {
  actions?: ReactNode;
}

/** Route actions portal into the one native shell renderer; common controls stay root-owned. */
export function BrowserTopBar({ actions }: BrowserTopBarProps) {
  const adopt = useContext(TopBar);
  const [slot, setSlot] = useState<ShellTopBar | null>(null);
  useLayoutEffect(() => {
    if (!adopt) throw new Error('BrowserTopBar requires a shell');
    setSlot(adopt());
  }, [adopt]);
  return slot ? createPortal(actions, slot.actions) : null;
}

const ShellBar = createContext<HTMLElement | null>(null);
/** The shell's header slot: a host renders its one header there (a portal), or in place when
 * there is no shell. */
export const useShellBar = () => useContext(ShellBar);

const ShellToolsContext = createContext<ShellTools | null>(null);
/** The bar's right-hand cluster (`search`, `chip`): a host renders its search control and its status
 * chip there (a portal), or in place when there is no shell. */
export const useShellTools = () => useContext(ShellToolsContext);

/** React hosts inside the one TMT web shell; the rail and menu use the shared static renderer. */
export function BrowserShell({ view, select, children, commonControls }: BrowserShellProps) {
  const root = useRef<HTMLDivElement>(null);
  const controls = useRef<ShellControls | null>(null);
  const [bar, setBar] = useState<HTMLDivElement | null>(null);
  const [search, setSearch] = useState<HTMLDivElement | null>(null);
  const [chip, setChip] = useState<HTMLDivElement | null>(null);
  const tools = useMemo(() => (search && chip ? { search, chip } : null), [search, chip]);
  const latest = useRef(select);
  const common = useRef(commonControls);
  common.current = commonControls;
  latest.current = select;
  useLayoutEffect(() => {
    controls.current = mountShell(root.current!, view, (id, href) =>
      Boolean(latest.current?.(id, href)),
    );
    controls.current.setCommonControls(common.current);
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
  useLayoutEffect(() => {
    controls.current?.setCommonControls(commonControls);
  }, [commonControls]);
  const adopt = useMemo(() => () => controls.current!.setTopBar(), []);
  return (
    <div ref={root} className="tmt-ui-shell">
      <div className="tmt-ui-shell-content">
        <div ref={setBar} className="tmt-ui-shell-bar" />
        <div className="tmt-ui-shell-tools">
          <div ref={setSearch} className="tmt-ui-shell-tools-search" />
          <div ref={setChip} className="tmt-ui-shell-tools-chip" />
        </div>
        <div className="tmt-ui-shell-main">
          {/* Children mount once the header and tool slots exist (the same commit, before paint),
              so a host header portals into them from its first mount and is never remounted. */}
          <ShellBar value={bar}>
            <ShellToolsContext.Provider value={tools}>
              <TopBar.Provider value={adopt}>{bar && tools ? children : null}</TopBar.Provider>
            </ShellToolsContext.Provider>
          </ShellBar>
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
  return (
    <ShellBar value={slot.bar}>
      <ShellToolsContext.Provider value={slot.tools}>
        <TopBar.Provider value={slot.setTopBar}>{children}</TopBar.Provider>
      </ShellToolsContext.Provider>
    </ShellBar>
  );
}
