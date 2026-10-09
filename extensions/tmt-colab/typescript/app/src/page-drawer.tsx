import { BrowserIconAction } from '@tmt/browser-ui/react';
import { createPortal } from 'react-dom';
import { X } from 'lucide-react';
import { useEffect, useId, useRef, type ReactNode } from 'react';

/** Parent chrome only. Closed drawers stay mounted to retain drafts and frozen attempts. */
export function PageDrawer({
  open,
  title,
  kind,
  close,
  children,
  hideHeader = false,
}: {
  open: boolean;
  title: string;
  kind: string;
  close(): void;
  children: ReactNode;
  hideHeader?: boolean;
}) {
  const label = useId();
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const node = dialog.current!;
    if (!open) return;
    const origin = document.activeElement;
    const mobile = matchMedia('(max-width: 640px)');
    const show = () => {
      // Changing native modality re-runs dialog focus steps; keep an active draft.
      const focused =
        node.open && node.contains(document.activeElement) ? document.activeElement : undefined;
      if (node.open) node.close();
      if (mobile.matches) node.showModal();
      else node.show();
      if (focused instanceof HTMLElement && focused.isConnected && node.contains(focused))
        focused.focus({ preventScroll: true });
    };
    show();
    mobile.addEventListener('change', show);
    return () => {
      mobile.removeEventListener('change', show);
      node.close();
      if (origin instanceof HTMLElement && origin.isConnected && origin.getClientRects().length)
        origin.focus();
      else document.querySelector<HTMLButtonElement>('.page-overflow-toggle button')?.focus();
    };
  }, [open]);
  return createPortal(
    <dialog
      ref={dialog}
      className="page-drawer"
      data-panel={kind}
      aria-labelledby={hideHeader ? undefined : label}
      aria-label={hideHeader ? title : undefined}
      onCancel={(event) => {
        event.preventDefault();
        close();
      }}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.preventDefault();
          close();
        }
      }}
    >
      {!hideHeader && (
        <header className="drawer-bar">
          <h2 id={label}>{title}</h2>
          <BrowserIconAction
            type="button"
            label={`Close ${title}`}
            variant="text"
            icon={<X />}
            onActivate={(event) => {
              if (event.isTrusted) close();
            }}
          />
        </header>
      )}
      <div className="drawer-body">{children}</div>
    </dialog>,
    document.body,
  );
}
