// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
import * as React from 'react';
import { cva, type VariantProps } from 'class-variance-authority';
import { Slot } from 'radix-ui';

import { cn } from './utils.js';

const badgeVariants = cva(
  'tmt:font-body tmt:[border-radius:var(--tmt-foundation-radius)] tmt:inline-flex tmt:w-fit tmt:shrink-0 tmt:items-center tmt:justify-center tmt:gap-1 tmt:overflow-hidden tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:border-transparent tmt:px-2 tmt:py-0.5 tmt:text-(length:--tmt-foundation-caption-size) tmt:font-(weight:--tmt-foundation-eyebrow-weight) tmt:whitespace-nowrap tmt:focus-visible:border-ring tmt:focus-visible:outline-solid tmt:focus-visible:outline-ring tmt:focus-visible:outline-(length:--tmt-foundation-focus-width) tmt:aria-invalid:border-destructive tmt:[&>svg]:pointer-events-none tmt:[&>svg]:size-3',
  {
    variants: {
      variant: {
        default: 'tmt:bg-primary tmt:text-primary-foreground tmt:[a&]:hover:bg-primary',
        secondary: 'tmt:bg-secondary tmt:text-secondary-foreground tmt:[a&]:hover:bg-secondary',
        destructive: 'tmt:bg-destructive tmt:text-background tmt:[a&]:hover:bg-destructive',
        outline:
          'tmt:border-border tmt:text-foreground tmt:[a&]:hover:bg-accent tmt:[a&]:hover:text-accent-foreground',
        ghost: 'tmt:[a&]:hover:bg-accent tmt:[a&]:hover:text-accent-foreground',
        link: 'tmt:text-foreground tmt:underline-offset-4 tmt:[a&]:hover:underline',
      },
    },
    defaultVariants: {
      variant: 'default',
    },
  },
);

function Badge({
  className,
  variant = 'default',
  asChild = false,
  ...props
}: React.ComponentProps<'span'> & VariantProps<typeof badgeVariants> & { asChild?: boolean }) {
  const Comp = asChild ? Slot.Root : 'span';

  return (
    <Comp
      data-slot="badge"
      data-variant={variant}
      className={cn(badgeVariants({ variant }), className)}
      {...props}
    />
  );
}

export { Badge, badgeVariants };
