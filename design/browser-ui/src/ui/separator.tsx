// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
'use client';

import * as React from 'react';
import { Separator as SeparatorPrimitive } from 'radix-ui';

import { cn } from './utils.js';

function Separator({
  className,
  orientation = 'horizontal',
  decorative = true,
  ...props
}: React.ComponentProps<typeof SeparatorPrimitive.Root>) {
  return (
    <SeparatorPrimitive.Root
      data-slot="separator"
      decorative={decorative}
      orientation={orientation}
      className={cn(
        'tmt:shrink-0 tmt:bg-border tmt:data-[orientation=horizontal]:h-(--tmt-foundation-rule-width) tmt:data-[orientation=horizontal]:w-full tmt:data-[orientation=vertical]:h-full tmt:data-[orientation=vertical]:w-(--tmt-foundation-rule-width)',
        className,
      )}
      {...props}
    />
  );
}

export { Separator };
