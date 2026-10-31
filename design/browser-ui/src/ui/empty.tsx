// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
import * as React from 'react';

import { cn } from './utils.js';

/** An empty state: a centred title and description inside a dashed rule, and room for one action. */
function Empty({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="empty"
      className={cn(
        'tmt:box-border tmt:font-body tmt:flex tmt:min-w-0 tmt:flex-1 tmt:flex-col tmt:items-center tmt:justify-center tmt:gap-4 tmt:rounded-surface tmt:border-dashed tmt:border-(length:--tmt-foundation-rule-width) tmt:border-border tmt:p-6 tmt:text-center tmt:text-balance tmt:text-foreground',
        className,
      )}
      {...props}
    />
  );
}

function EmptyHeader({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="empty-header"
      className={cn('tmt:flex tmt:max-w-sm tmt:flex-col tmt:items-center tmt:gap-2', className)}
      {...props}
    />
  );
}

function EmptyMedia({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="empty-icon"
      className={cn(
        'tmt:mb-1 tmt:flex tmt:size-10 tmt:shrink-0 tmt:items-center tmt:justify-center tmt:rounded-control tmt:bg-muted tmt:text-foreground tmt:[&_svg]:pointer-events-none tmt:[&_svg]:size-5',
        className,
      )}
      {...props}
    />
  );
}

function EmptyTitle({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="empty-title"
      className={cn(
        'tmt:text-(length:--tmt-foundation-command-size) tmt:font-(weight:--tmt-foundation-eyebrow-weight)',
        className,
      )}
      {...props}
    />
  );
}

function EmptyDescription({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="empty-description"
      className={cn(
        'tmt:text-(length:--tmt-foundation-caption-size) tmt:text-muted-foreground',
        className,
      )}
      {...props}
    />
  );
}

function EmptyContent({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="empty-content"
      className={cn(
        'tmt:flex tmt:w-full tmt:max-w-sm tmt:min-w-0 tmt:flex-col tmt:items-center tmt:gap-2',
        className,
      )}
      {...props}
    />
  );
}

export { Empty, EmptyHeader, EmptyTitle, EmptyDescription, EmptyContent, EmptyMedia };
