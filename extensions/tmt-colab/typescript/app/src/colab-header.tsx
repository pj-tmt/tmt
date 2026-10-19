import { RemoteHeaderChip, RemoteMoreMenu, useRemoteHeaderAvailable } from './remote-header.js';
import type { ReactNode, Ref } from 'react';
import { BrowserHeader } from '@tmt/browser-ui/react';
import { text } from './strings.js';
import './colab-header.css';
import { QuickSwitcherControl } from './quick-switcher-control.js';

/** Colab supplies routing and content to the shared presentation. */
export function ColabHeader({
  title,
  caption,
  actions,
  home,
  headerRef,
  status,
  remote,
  remoteMenu = true,
  crumb,
  headingSuffix,
}: {
  title: string;
  caption?: string;
  actions?: ReactNode;
  /** Wraps the brand in the page's home link; `name` is its accessible name and tooltip. */
  home?: (brand: ReactNode, name: string) => ReactNode;
  headerRef?: Ref<HTMLElement>;
  status?: ReactNode;
  remote?: { deviceName?: string };
  remoteMenu?: boolean;
  /** A crumb between the brand and the title, with its own separator. */
  crumb?: ReactNode;
  /** A crumb or control after the title, supplied with its own separator. */
  headingSuffix?: ReactNode;
}) {
  const remoteAvailable = useRemoteHeaderAvailable();
  const deviceName = remote?.deviceName?.trim();
  // The connection identity names the brand: no visible element beside the mark and the wordmark.
  const identity =
    remote === undefined
      ? undefined
      : text.brandIdentity(deviceName ? `local · ${deviceName}` : 'local');
  return (
    <BrowserHeader
      productLabel={text.product}
      headingPrefix={
        remote === undefined ? undefined : (
          <>
            <span className="colab-title-divider" aria-hidden="true">
              /
            </span>
            {crumb !== undefined && (
              <span className="colab-crumb-group">
                {crumb}
                <span className="colab-title-divider" aria-hidden="true">
                  /
                </span>
              </span>
            )}
          </>
        )
      }
      headingSuffix={headingSuffix}
      title={title}
      caption={caption}
      captionId={caption === undefined ? undefined : 'colab-header-caption'}
      brandLink={
        home
          ? (brand) => home(brand, identity ?? text.home)
          : identity === undefined
            ? undefined
            : (brand) => (
                <span className="colab-brand" role="group" aria-label={identity} title={identity}>
                  {brand}
                </span>
              )
      }
      actions={
        <>
          <QuickSwitcherControl />
          {actions}
          {remoteMenu && (
            <span className="remote-home-more">
              <RemoteMoreMenu />
            </span>
          )}
        </>
      }
      status={
        remoteAvailable || status !== undefined ? (
          <>
            <RemoteHeaderChip />
            {status}
          </>
        ) : undefined
      }
      headerRef={headerRef}
    />
  );
}
