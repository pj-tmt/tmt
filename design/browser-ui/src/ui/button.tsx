// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
import * as React from 'react';
import { cva, type VariantProps } from 'class-variance-authority';
import { Slot } from 'radix-ui';

import { cn } from './utils.js';

const buttonVariants = cva(
  "tmt:bg-transparent tmt:appearance-none tmt:[border-radius:var(--tmt-foundation-radius)] tmt:font-body tmt:m-0 tmt:box-border tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:border-transparent tmt:inline-flex tmt:shrink-0 tmt:items-center tmt:justify-center tmt:gap-2 tmt:text-(length:--tmt-foundation-command-size) tmt:font-(weight:--tmt-foundation-eyebrow-weight) tmt:whitespace-nowrap tmt:focus-visible:border-ring tmt:focus-visible:outline-solid tmt:focus-visible:outline-ring tmt:focus-visible:outline-(length:--tmt-foundation-focus-width) tmt:disabled:pointer-events-none tmt:disabled:opacity-50 tmt:aria-invalid:border-destructive tmt:[&_svg]:pointer-events-none tmt:[&_svg]:shrink-0 tmt:[&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default: 'tmt:bg-primary tmt:text-primary-foreground tmt:hover:bg-primary',
        destructive: 'tmt:bg-destructive tmt:text-background tmt:hover:bg-destructive',
        outline:
          'tmt:border-input tmt:bg-background tmt:hover:bg-accent tmt:hover:text-accent-foreground',
        secondary:
          'tmt:border-input tmt:bg-secondary tmt:text-secondary-foreground tmt:hover:bg-secondary',
        ghost: 'tmt:hover:bg-accent tmt:hover:text-accent-foreground',
        link: 'tmt:text-foreground tmt:underline-offset-4 tmt:hover:underline',
      },
      size: {
        default: 'tmt:h-9 tmt:px-4 tmt:py-2 tmt:has-[>svg]:px-3',
        xs: "tmt:h-6 tmt:gap-1 tmt:px-2 tmt:text-(length:--tmt-foundation-caption-size) tmt:has-[>svg]:px-1.5 tmt:[&_svg:not([class*='size-'])]:size-3",
        sm: 'tmt:h-8 tmt:gap-1.5 tmt:px-3 tmt:has-[>svg]:px-2.5',
        lg: 'tmt:h-10 tmt:px-6 tmt:has-[>svg]:px-4',
        icon: 'tmt:size-9',
        'icon-xs': "tmt:size-6 tmt:[&_svg:not([class*='size-'])]:size-3",
        'icon-sm': 'tmt:size-8',
        'icon-lg': 'tmt:size-10',
      },
    },
    defaultVariants: {
      variant: 'default',
      size: 'default',
    },
  },
);

function Button({
  className,
  variant = 'default',
  size = 'default',
  asChild = false,
  busy = false,
  busyLabel,
  disabled,
  children,
  onClickCapture,
  onKeyDownCapture,
  ...props
}: React.ComponentProps<'button'> &
  VariantProps<typeof buttonVariants> & {
    asChild?: boolean;
  } & ({ busy?: false; busyLabel?: string } | { busy: true; busyLabel: string })) {
  const Comp = asChild ? Slot.Root : 'button';
  const blocked = disabled || busy;
  if (busy && !busyLabel) throw new Error('Busy Button requires a visible busyLabel');
  if (asChild && busy) throw new Error('Busy Button uses its native button, not asChild');

  // Slot merges child callbacks first. Fence the actual target before that composition.
  const fenceCapture = (child: React.ReactNode): React.ReactNode => {
    if (!React.isValidElement<React.HTMLAttributes<HTMLElement>>(child)) return child;
    if (child.type === Slot.Slottable) {
      const slottable = child as React.ReactElement<React.ComponentProps<typeof Slot.Slottable>>;
      return 'child' in slottable.props
        ? React.cloneElement(slottable, { child: fenceCapture(slottable.props.child) })
        : React.cloneElement(slottable, { children: fenceCapture(slottable.props.children) });
    }
    return React.cloneElement(child, {
      onClickCapture: (event) => {
        event.preventDefault();
        event.stopPropagation();
      },
      onKeyDownCapture: (event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          event.stopPropagation();
        } else child.props.onKeyDownCapture?.(event);
      },
    });
  };
  const content =
    asChild && blocked
      ? React.isValidElement(children)
        ? fenceCapture(children)
        : React.Children.map(children, fenceCapture)
      : children;

  return (
    <Comp
      data-slot="button"
      data-variant={variant}
      data-size={size}
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
      disabled={blocked}
      aria-disabled={blocked || undefined}
      aria-busy={busy || undefined}
      onClickCapture={(event) => {
        if (blocked) {
          event.preventDefault();
          event.stopPropagation();
        } else onClickCapture?.(event);
      }}
      onKeyDownCapture={(event) => {
        if (blocked && (event.key === 'Enter' || event.key === ' ')) {
          event.preventDefault();
          event.stopPropagation();
        } else onKeyDownCapture?.(event);
      }}
    >
      {busy ? busyLabel : content}
    </Comp>
  );
}

export { Button, buttonVariants };
