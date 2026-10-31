// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
import * as React from 'react';

import { cn } from './utils.js';

/** A quiet placeholder block in the muted surface. It does not pulse: the busy state is the leaf's
 * BusyMark, so a skeleton never adds motion of its own. */
function Skeleton({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="skeleton"
      aria-hidden="true"
      className={cn('tmt:rounded-small tmt:bg-muted', className)}
      {...props}
    />
  );
}

export { Skeleton };
