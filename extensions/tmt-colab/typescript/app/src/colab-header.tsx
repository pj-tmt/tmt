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
}: {
  title: string;
  caption?: string;
  actions?: ReactNode;
  home?: (brand: ReactNode) => ReactNode;
  headerRef?: Ref<HTMLElement>;
  status?: ReactNode;
  remote?: { deviceName?: string };
  remoteMenu?: boolean;
}) {
  const remoteAvailable = useRemoteHeaderAvailable();
  const deviceName = remote?.deviceName?.trim();
  const remoteLabel = deviceName ? `local · ${deviceName}` : 'local';
  return (
    <BrowserHeader
      productLabel={text.product}
      leading={
        remote === undefined ? undefined : (
          <span
            className="colab-remote-attribution"
            role="group"
            aria-label={text.remoteAttribution(remoteLabel)}
            title={text.remoteAttribution(remoteLabel)}
          >
            <span aria-hidden="true">◉</span>
            <span className="colab-remote-label" aria-hidden="true">
              {remoteLabel}
            </span>
          </span>
        )
      }
      headingPrefix={
        remote === undefined ? undefined : (
          <span className="colab-title-divider" aria-hidden="true">
            /
          </span>
        )
      }
      title={title}
      caption={caption}
      captionId={caption === undefined ? undefined : 'colab-header-caption'}
      brandLink={home}
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
