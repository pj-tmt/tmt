import {
  useId,
  useLayoutEffect,
  useRef,
  type ComponentPropsWithoutRef,
  type ReactNode,
  type SyntheticEvent,
} from 'react';
import { DrawerHeader } from './drawer-header.js';
import { admitsOverlayEscape, restoreOverlayFocus } from './overlay.js';

export type BrowserPanelDismissReason = 'escape' | 'close';
export interface BrowserPanelProps extends Omit<
  ComponentPropsWithoutRef<'aside'>,
  'title' | 'children' | 'onKeyDown'
> {
  open: boolean;
  title: string;
  caption?: ReactNode;
  /** The host chooses the layout; column geometry and reflow belong to its page grid. */
  mode: 'column' | 'card' | 'sheet';
  children: ReactNode;
  onDismiss(reason: BrowserPanelDismissReason, event: SyntheticEvent): void;
  focusFallback?(): HTMLElement | null;
}

/** A controlled non-modal region: no top layer, outside-press handler, trap or page inertness. */
export function BrowserPanel({
  open,
  title,
  caption,
  mode,
  children,
  onDismiss,
  focusFallback,
  className = '',
  ...attributes
}: BrowserPanelProps) {
  const label = useId();
  const heading = useRef<HTMLHeadingElement>(null);
  const current = useRef({ focusFallback });
  current.current = { focusFallback };
  useLayoutEffect(() => {
    const node = heading.current;
    if (!open || !node) return;
    const origin = node.ownerDocument.activeElement;
    node.focus({ preventScroll: true });
    return () => restoreOverlayFocus(origin, current.current.focusFallback);
  }, [open]);
  if (!open) return null;
  return (
    <aside
      {...attributes}
      className={`tmt-ui-panel ${className}`}
      data-mode={mode}
      aria-labelledby={label}
      onKeyDown={(event) => {
        if (
          event.key !== 'Escape' ||
          event.defaultPrevented ||
          !event.currentTarget.contains(event.target as Node) ||
          !admitsOverlayEscape(event)
        )
          return;
        event.preventDefault();
        event.stopPropagation();
        onDismiss('escape', event);
      }}
    >
      <DrawerHeader
        title={title}
        caption={caption}
        label={label}
        headingRef={heading}
        focusHeading
        onClose={(event) => onDismiss('close', event)}
      />
      <div className="tmt-ui-drawer-body">{children}</div>
    </aside>
  );
}
