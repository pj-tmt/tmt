// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
import * as React from 'react';
import { cva, type VariantProps } from 'class-variance-authority';

import { cn } from './utils.js';

const alertVariants = cva(
  'tmt:font-body tmt:[border-radius:var(--tmt-foundation-radius)] tmt:border-border tmt:relative tmt:grid tmt:w-full tmt:grid-cols-[0_1fr] tmt:items-start tmt:gap-y-0.5 tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:px-4 tmt:py-3 tmt:text-(length:--tmt-foundation-command-size) tmt:has-[>svg]:grid-cols-[calc(var(--tmt-foundation-spacing-unit)*4)_1fr] tmt:has-[>svg]:gap-x-3 tmt:[&>svg]:size-4 tmt:[&>svg]:translate-y-0.5 tmt:[&>svg]:text-current',
  {
    variants: {
      variant: {
        default: 'tmt:bg-card tmt:text-card-foreground',
        destructive:
          'tmt:bg-card tmt:text-destructive tmt:*:data-[slot=alert-description]:text-destructive tmt:[&>svg]:text-current',
      },
    },
    defaultVariants: {
      variant: 'default',
    },
  },
);

function Alert({
  className,
  variant,
  ...props
}: React.ComponentProps<'div'> & VariantProps<typeof alertVariants>) {
  return (
    <div
      data-slot="alert"
      role="alert"
      className={cn(alertVariants({ variant }), className)}
      {...props}
    />
  );
}

function AlertTitle({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="alert-title"
      className={cn(
        'tmt:col-start-2 tmt:line-clamp-1 tmt:min-h-4 tmt:font-(weight:--tmt-foundation-eyebrow-weight)',
        className,
      )}
      {...props}
    />
  );
}

function AlertDescription({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="alert-description"
      className={cn(
        'tmt:col-start-2 tmt:grid tmt:justify-items-start tmt:gap-1 tmt:text-(length:--tmt-foundation-command-size) tmt:text-muted-foreground',
        className,
      )}
      {...props}
    />
  );
}

export { Alert, AlertTitle, AlertDescription };
