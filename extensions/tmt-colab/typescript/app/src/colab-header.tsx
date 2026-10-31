import { RemoteHeaderChip, RemoteMoreMenu, useRemoteHeaderAvailable } from './remote-header.js';
import {
  useContext,
  useLayoutEffect,
  useSyncExternalStore,
  type ReactNode,
  type Ref,
  type RefCallback,
} from 'react';
import {
  BrowserHeader,
  BrowserMenu,
  BrowserTopBar,
  useShellBar,
  useShellTools,
} from '@tmt/browser-ui/react';
import { createPortal } from 'react-dom';
import { text } from './strings.js';
import './colab-header.css';
import { QuickSwitcherControl } from './quick-switcher-control.js';
import { SharedTopBar } from './shared-top-bar.js';
import { getThemeChoice, subscribeTheme } from './theme.js';
import { setStoredTheme, THEMES } from '@tmt/browser-ui/theme';
import { Sun } from 'lucide-react';

const captionId = 'colab-header-caption';

/** Moves focus to the route's heading after an in-app navigation: in the shared bar's title slot,
 * else in the module's own header. */
export function focusPageHeading(): void {
  const title =
    document.querySelector<HTMLElement>('.tmt-ui-topbar-title h1') ??
    document.querySelector<HTMLElement>('.tmt-ui-header h1');
  if (!title) return;
  title.tabIndex = -1;
  title.focus({ preventScroll: true });
}

function assign<T>(ref: Ref<T> | undefined, value: T | null) {
  if (typeof ref === 'function') ref(value);
  else if (ref) ref.current = value;
}
/** One element for two refs: the header is both the title's container and, outside the shared bar,
 * the sticky chrome. */
function both<T>(
  first: Ref<T> | undefined,
  second: Ref<T> | undefined,
): RefCallback<T> | undefined {
  if (!first && !second) return undefined;
  return (node) => {
    assign(first, node);
    assign(second, node);
  };
}

/** The mark and the wordmark as one flex row; under 480px the wordmark is visually hidden (the brand's
 * accessible name still carries the product). */
function BrandParts({ children }: { children: ReactNode }) {
  return <span className="colab-brand-parts">{children}</span>;
}

/** Colab supplies routing and content to the shared presentation. Inside the TMT web shell the
 * header renders into the shell's one header slot, wherever its route draws it. */
export function ColabHeader({
  title,
  titleVisible = true,
  caption,
  actions,
  home,
  headerRef,
  barRef,
  page = false,
  status,
  remote,
  remoteMenu = true,
  remoteChip = true,
  crumb,
  headingSuffix,
  embedded = false,
  reader = false,
  brandOnly = false,
}: {
  title: string;
  titleVisible?: boolean;
  caption?: string;
  actions?: ReactNode;
  /** Wraps the brand in the page's home link; `name` is its accessible name and tooltip. */
  home?: (brand: ReactNode, name: string) => ReactNode;
  headerRef?: Ref<HTMLElement>;
  /** The sticky chrome the page measures from (the header; in the shared bar, the bar itself). */
  barRef?: Ref<HTMLElement>;
  /** The title names a page: in the shared bar its crumb (the scope) leads it, not the module. */
  page?: boolean;
  status?: ReactNode;
  remote?: { deviceName?: string };
  remoteMenu?: boolean;
  /** The Remote count chip in the status slot; the page header keeps only an attention mark. */
  remoteChip?: boolean;
  /** A crumb between the brand and the title, with its own separator. */
  crumb?: ReactNode;
  /** A crumb or control after the title, supplied with its own separator. */
  headingSuffix?: ReactNode;
  /** The page is hosted inside another surface that already names it: only its actions and status
   * remain, in place, never in the host's header slot. */
  embedded?: boolean;
  reader?: boolean;
  brandOnly?: boolean;
}) {
  const remoteAvailable = useRemoteHeaderAvailable();
  const deviceName = remote?.deviceName?.trim();
  // The connection identity names the brand: no visible element beside the mark and the wordmark.
  const identity =
    remote === undefined
      ? undefined
      : text.brandIdentity(deviceName ? `local · ${deviceName}` : 'local');
  const bar = useShellBar();
  const tools = useShellTools();
  const shared = useContext(SharedTopBar) && bar !== null;
  if (embedded)
    return (
      <header className="colab-embedded-bar" ref={headerRef} aria-label={title}>
        {actions}
        {status}
      </header>
    );
  // In the shell the rail names the module and the search and the presence chip sit in the bar's
  // own cluster; the header keeps the page's title, status and actions.
  const clustered = !shared && bar !== null && tools !== null;
  useLayoutEffect(() => {
    if (!shared) return;
    assign(barRef, bar);
    return () => assign(barRef, null);
  }, [shared, barRef, bar]);
  const chip = remoteChip && remoteAvailable ? <RemoteHeaderChip /> : null;
  const header = (
    <BrowserHeader
      productLabel={bar ? (identity ?? text.product) : text.product}
      inShell={bar !== null}
      headingPrefix={
        bar !== null ? (
          remote !== undefined && crumb !== undefined ? (
            <CrumbGroup>{crumb}</CrumbGroup>
          ) : undefined
        ) : remote === undefined ? undefined : (
          <>
            <span className="colab-title-divider" aria-hidden="true">
              /
            </span>
            {crumb !== undefined && <CrumbGroup>{crumb}</CrumbGroup>}
          </>
        )
      }
      headingSuffix={headingSuffix}
      title={title}
      titleVisible={titleVisible}
      caption={caption}
      captionId={caption === undefined ? undefined : captionId}
      brandLink={
        home
          ? (brand) => home(<BrandParts>{brand}</BrandParts>, identity ?? text.home)
          : identity === undefined
            ? (brand) => <BrandParts>{brand}</BrandParts>
            : (brand) => (
                <span className="colab-brand" role="group" aria-label={identity} title={identity}>
                  <BrandParts>{brand}</BrandParts>
                </span>
              )
      }
      actions={
        shared || brandOnly ? undefined : (
          <>
            {!clustered && !reader && <QuickSwitcherControl />}
            {reader && <ReaderTheme />}
            {actions}
            {remoteMenu && !reader && (
              <span className="remote-home-more">
                <RemoteMoreMenu />
              </span>
            )}
          </>
        )
      }
      status={
        shared ? undefined : (!clustered && chip) || status !== undefined ? (
          <>
            {!clustered && chip}
            {status}
          </>
        ) : undefined
      }
      headerRef={shared ? headerRef : both(headerRef, barRef)}
    />
  );
  if (!bar) return header;
  if (shared)
    // The bar's title slot is the route's one heading: the module or the page's scope, the title,
    // then the page's chips. The content repeats none of it.
    return (
      <BrowserTopBar
        title={
          // The connection identity names the title's group for assistive technology, as the
          // module's own header names itself; nothing of it shows.
          <div
            className="colab-topbar-identity"
            role="group"
            aria-label={`${identity ?? text.product} · ${title}`}
          >
            <h1
              ref={(node) => assign(headerRef, node)}
              aria-describedby={caption === undefined ? undefined : captionId}
            >
              {(!page || crumb !== undefined) && (
                <span className="tmt-ui-topbar-title-prefix">
                  {page ? (
                    <CrumbGroup>{crumb}</CrumbGroup>
                  ) : (
                    <>
                      <CrumbGroup>{text.product}</CrumbGroup>
                      {crumb !== undefined && <CrumbGroup>{crumb}</CrumbGroup>}
                    </>
                  )}
                </span>
              )}
              <span className="tmt-ui-topbar-title-current" title={title}>
                {title}
              </span>
            </h1>
            {(headingSuffix !== undefined || caption !== undefined) && (
              <div className="tmt-ui-topbar-title-chips">
                {headingSuffix}
                {caption !== undefined && (
                  <span className="tmt-ui-caption" id={captionId} title={caption}>
                    {caption}
                  </span>
                )}
              </div>
            )}
          </div>
        }
        status={status}
        actions={actions}
      />
    );
  return (
    <>
      {createPortal(header, bar)}
      {tools && createPortal(<QuickSwitcherControl />, tools.search)}
      {tools && chip && createPortal(chip, tools.chip)}
    </>
  );
}

/** A crumb before the title, with its own separator after it. */
function CrumbGroup({ children }: { children: ReactNode }) {
  return (
    <span className="colab-crumb-group">
      {children}
      <span className="colab-title-divider" aria-hidden="true">
        /
      </span>
    </span>
  );
}

/** Reader links have no shell, so they expose the same stored theme menu locally. */
function ReaderTheme() {
  const choice = useSyncExternalStore(subscribeTheme, getThemeChoice, () => 'system' as const);
  return (
    <BrowserMenu
      label={`Theme: ${choice}`}
      icon={<Sun aria-hidden />}
      items={THEMES.map((key) => ({
        key,
        label: key === 'system' ? 'System' : key === 'light' ? 'Light' : 'Dark',
        checked: choice === key,
      }))}
      onSelect={(key) => setStoredTheme(key)}
    />
  );
}
