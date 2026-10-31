// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
import * as React from 'react';

import { cn } from './utils.js';

/** A resting surface: the sheet fill and the rule, the surface radius and elevation 1 (#5821). */
function Card({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="card"
      className={cn(
        'tmt:box-border tmt:font-body tmt:flex tmt:flex-col tmt:gap-4 tmt:rounded-surface tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:border-border tmt:bg-card tmt:shadow-elevation-1 tmt:py-4 tmt:text-card-foreground',
        className,
      )}
      {...props}
    />
  );
}

function CardHeader({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="card-header"
      className={cn(
        'tmt:grid tmt:auto-rows-min tmt:grid-rows-[auto_auto] tmt:items-start tmt:gap-1 tmt:px-4 tmt:has-data-[slot=card-action]:grid-cols-[1fr_auto]',
        className,
      )}
      {...props}
    />
  );
}

function CardTitle({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="card-title"
      className={cn(
        'tmt:leading-none tmt:text-(length:--tmt-foundation-command-size) tmt:font-(weight:--tmt-foundation-eyebrow-weight)',
        className,
      )}
      {...props}
    />
  );
}

function CardDescription({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="card-description"
      className={cn(
        'tmt:text-(length:--tmt-foundation-caption-size) tmt:text-muted-foreground',
        className,
      )}
      {...props}
    />
  );
}

function CardAction({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="card-action"
      className={cn(
        'tmt:col-start-2 tmt:row-span-2 tmt:row-start-1 tmt:self-start tmt:justify-self-end',
        className,
      )}
      {...props}
    />
  );
}

function CardContent({ className, ...props }: React.ComponentProps<'div'>) {
  return <div data-slot="card-content" className={cn('tmt:px-4', className)} {...props} />;
}

function CardFooter({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="card-footer"
      className={cn('tmt:flex tmt:items-center tmt:gap-2 tmt:px-4', className)}
      {...props}
    />
  );
}

export { Card, CardHeader, CardFooter, CardTitle, CardAction, CardDescription, CardContent };
