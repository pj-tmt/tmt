import type { ReactNode, Ref, MouseEvent } from 'react';
import { X } from 'lucide-react';
import { BrowserIconAction } from './icon-action.js';

/** The same heading, position caption and Close control for modal and non-modal surfaces. */
export function DrawerHeader({
  title,
  caption,
  headerActions,
  label,
  headingRef,
  focusHeading,
  closeLabel,
  closeTooltipLabel,
  onClose,
}: {
  title: string;
  caption?: ReactNode;
  headerActions?: ReactNode;
  label: string;
  headingRef?: Ref<HTMLHeadingElement>;
  focusHeading?: boolean;
  closeLabel?: string;
  closeTooltipLabel?: string;
  onClose(event: MouseEvent<HTMLButtonElement>): void;
}) {
  return (
    <header className="tmt-ui-drawer-header">
      <div className="tmt-ui-drawer-heading">
        <h2 id={label} ref={headingRef} tabIndex={focusHeading ? -1 : undefined}>
          {title}
        </h2>
        {caption && <div className="tmt-ui-drawer-caption">{caption}</div>}
      </div>
      {headerActions !== undefined && (
        <div className="tmt-ui-drawer-header-actions">{headerActions}</div>
      )}
      <BrowserIconAction
        type="button"
        variant="text"
        label={closeLabel ?? `Close ${title}`}
        tooltipLabel={closeTooltipLabel ?? `Close ${title} · Esc`}
        ariaKeyShortcuts="Escape"
        icon={<X />}
        onActivate={(event) => {
          if (event.isTrusted) onClose(event);
        }}
      />
    </header>
  );
}
