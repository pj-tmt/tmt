import * as React from 'react';
import { cva, type VariantProps } from 'class-variance-authority';

import { cn } from './utils.js';

/** The surface levels (#5821), one per place a surface sits; each pairs a fill with a hairline edge
 * and one level of the elevation scale, since dark themes show shadows weakly. The layers use the
 * same levels: Card is a sheet, the menus, Select, Popover and Tooltip are overlays, Dialog and Sheet
 * are modal, so one shadow token changes all of them. */
const surfaceVariants = cva('tmt:box-border tmt:font-body tmt:text-foreground', {
  variants: {
    level: {
      // The page itself: no edge, no shadow.
      paper: 'tmt:bg-background',
      // A card or panel resting on the page.
      sheet:
        'tmt:rounded-surface tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:border-border tmt:bg-card tmt:shadow-elevation-1',
      // A surface lifted while hovered or dragged.
      raised:
        'tmt:rounded-surface tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:border-border tmt:bg-card tmt:shadow-overlay',
      // A floating layer: popover, menu, tooltip.
      overlay:
        'tmt:rounded-control tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:border-layer-boundary tmt:bg-popover tmt:shadow-overlay',
      // A dialog or sheet above the scrim.
      modal:
        'tmt:rounded-surface tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:border-layer-boundary tmt:bg-popover tmt:shadow-elevation-3',
    },
  },
  defaultVariants: { level: 'sheet' },
});

function Surface({
  className,
  level,
  ...props
}: React.ComponentProps<'div'> & VariantProps<typeof surfaceVariants>) {
  return (
    <div
      data-slot="surface"
      data-level={level ?? 'sheet'}
      className={cn(surfaceVariants({ level }), className)}
      {...props}
    />
  );
}

export { Surface, surfaceVariants };
