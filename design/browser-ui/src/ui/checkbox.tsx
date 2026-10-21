// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
'use client';

import * as React from 'react';
import { Checkbox as CheckboxPrimitive } from 'radix-ui';

import { cn } from './utils.js';

function Checkbox({
  className,
  readOnly = false,
  onClick,
  ...props
}: React.ComponentProps<typeof CheckboxPrimitive.Root> & { readOnly?: boolean }) {
  return (
    <CheckboxPrimitive.Root
      data-slot="checkbox"
      // The shared static drawing (`tmt-ui-checkbox` in static.css) keyed on Radix's data-state;
      // one look for native and Radix checkboxes.
      className={cn('tmt-ui-checkbox', className)}
      {...props}
      aria-readonly={readOnly || undefined}
      onClick={(event) => {
        if (readOnly) event.preventDefault();
        else onClick?.(event);
      }}
    />
  );
}

export { Checkbox };
