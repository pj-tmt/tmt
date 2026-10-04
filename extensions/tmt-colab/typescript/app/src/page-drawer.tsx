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
}: {
  open: boolean;
  title: string;
  kind: string;
  close(): void;
  children: ReactNode;
}) {
  const label = useId();
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const node = dialog.current!;
    if (!open) return;
    const origin = document.activeElement;
    const mobile = matchMedia('(max-width: 640px)');
    const show = () => {
      if (node.open) node.close();
      if (mobile.matches) node.showModal();
      else node.show();
    };
    show();
    mobile.addEventListener('change', show);
    return () => {
      mobile.removeEventListener('change', show);
      node.close();
      if (origin instanceof HTMLElement && origin.isConnected && origin.getClientRects().length)
        origin.focus();
      else document.querySelector<HTMLButtonElement>('.page-overflow-toggle')?.focus();
    };
  }, [open]);
  return createPortal(
    <dialog
      ref={dialog}
      className="page-drawer"
      data-panel={kind}
      aria-labelledby={label}
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
      <header className="drawer-bar">
        <h2 id={label}>{title}</h2>
        <button
          type="button"
          aria-label={`Close ${title}`}
          onClick={(event) => {
            if (event.isTrusted) close();
          }}
        >
          <X aria-hidden />
        </button>
      </header>
      <div className="drawer-body">{children}</div>
    </dialog>,
    document.body,
  );
}
