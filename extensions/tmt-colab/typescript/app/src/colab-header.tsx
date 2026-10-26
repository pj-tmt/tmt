import { RemoteHeaderChip, RemoteMoreMenu, useRemoteHeaderAvailable } from './remote-header.js';
import { useContext, useLayoutEffect, type ReactNode, type Ref, type RefCallback } from 'react';
import { BrowserHeader, BrowserTopBar, useShellBar, useShellTools } from '@tmt/browser-ui/react';
import { createPortal } from 'react-dom';
import { text } from './strings.js';
import './colab-header.css';
import { QuickSwitcherControl } from './quick-switcher-control.js';
import { SharedTopBar } from './shared-top-bar.js';

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
  status,
  remote,
  remoteMenu = true,
  remoteChip = true,
  crumb,
  headingSuffix,
  embedded = false,
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
      captionId={caption === undefined ? undefined : 'colab-header-caption'}
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
        shared ? undefined : (
          <>
            {!clustered && <QuickSwitcherControl />}
            {actions}
            {remoteMenu && (
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
  if (shared) {
    // The bar draws no title: the page's own row (title, crumb, project menu, caption) is content that
    // scrolls with it, and only its actions and status belong to the shared bar.
    const row = titleVisible || headingSuffix !== undefined || caption !== undefined;
    return (
      <>
        <BrowserTopBar
          actions={
            <>
              {status}
              {actions}
            </>
          }
        />
        {row && <div className="colab-title-row">{header}</div>}
      </>
    );
  }
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
