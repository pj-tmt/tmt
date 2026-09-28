import { useCallback, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import './extension-panel.css';

/** Shared native modal lifetime; visited content retains drafts when closed. */
export function useExtensionPanel(
  label: string,
  content: ReactNode | ((open: boolean) => ReactNode),
  className = ''
) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [visited, setVisited] = useState(false);
  const [visible, setVisible] = useState(false);
  const open = useCallback(() => {
    setVisited(true);
    dialog.current?.showModal();
    setVisible(true);
  }, []);
  return {
    open,
    panel: (
      <dialog
        ref={dialog}
        className={`extension-panel ${className}`}
        aria-label={label}
        onClose={() => setVisible(false)}
        onKeyDown={(event) => {
          // Escape closes the native dialog, not the selected room behind it.
          if (event.key === 'Escape') event.stopPropagation();
        }}
      >
        <button
          type="button"
          className="extension-panel-close"
          aria-label={`Close ${label.toLowerCase()}`}
          onClick={() => dialog.current?.close()}
        >
          ×
        </button>
        {visited && (typeof content === 'function' ? content(visible) : content)}
      </dialog>
    ),
  };
}
