// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
import * as React from 'react';

import { cn } from './utils.js';

/** A quiet placeholder block in the muted surface with a calm opacity pulse (#5821), never a shimmer
 * sweep; under reduced motion it holds still. */
function Skeleton({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="skeleton"
      aria-hidden="true"
      className={cn('tmt:rounded-small tmt:bg-muted tmt:animate-pulse-calm', className)}
      {...props}
    />
  );
}

export { Skeleton };
