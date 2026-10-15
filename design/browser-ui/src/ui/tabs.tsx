// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
'use client';

import * as React from 'react';
import { cva, type VariantProps } from 'class-variance-authority';
import { Tabs as TabsPrimitive } from 'radix-ui';

import { cn } from './utils.js';

function Tabs({
  className,
  orientation = 'horizontal',
  ...props
}: React.ComponentProps<typeof TabsPrimitive.Root>) {
  return (
    <TabsPrimitive.Root
      data-slot="tabs"
      data-orientation={orientation}
      orientation={orientation}
      className={cn(
        'tmt:font-body tmt:group/tabs tmt:flex tmt:gap-2 tmt:data-[orientation=horizontal]:flex-col',
        className,
      )}
      {...props}
    />
  );
}

const tabsListVariants = cva(
  'tmt:[border-radius:var(--tmt-foundation-radius)] tmt:group/tabs-list tmt:inline-flex tmt:w-fit tmt:items-center tmt:justify-center tmt:p-1 tmt:text-muted-foreground tmt:group-data-[orientation=horizontal]/tabs:h-9 tmt:group-data-[orientation=vertical]/tabs:h-fit tmt:group-data-[orientation=vertical]/tabs:flex-col',
  {
    variants: {
      variant: {
        default: 'tmt:bg-muted',
        line: 'tmt:gap-1 tmt:bg-transparent',
      },
    },
    defaultVariants: {
      variant: 'default',
    },
  },
);

function TabsList({
  className,
  variant = 'default',
  ...props
}: React.ComponentProps<typeof TabsPrimitive.List> & VariantProps<typeof tabsListVariants>) {
  return (
    <TabsPrimitive.List
      data-slot="tabs-list"
      data-variant={variant}
      className={cn(tabsListVariants({ variant }), className)}
      {...props}
    />
  );
}

function TabsTrigger({ className, ...props }: React.ComponentProps<typeof TabsPrimitive.Trigger>) {
  return (
    <TabsPrimitive.Trigger
      data-slot="tabs-trigger"
      className={cn(
        "tmt:bg-transparent tmt:appearance-none tmt:[border-radius:var(--tmt-foundation-radius)] tmt:font-body tmt:m-0 tmt:box-border tmt:relative tmt:inline-flex tmt:h-[calc(100%-var(--tmt-foundation-rule-width))] tmt:flex-1 tmt:items-center tmt:justify-center tmt:gap-1.5 tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:border-transparent tmt:px-2 tmt:py-1 tmt:text-(length:--tmt-foundation-command-size) tmt:font-(weight:--tmt-foundation-eyebrow-weight) tmt:whitespace-nowrap tmt:text-foreground tmt:group-data-[orientation=vertical]/tabs:w-full tmt:group-data-[orientation=vertical]/tabs:justify-start tmt:hover:text-foreground tmt:focus-visible:border-ring tmt:focus-visible:outline-solid tmt:focus-visible:outline-ring tmt:focus-visible:outline-(length:--tmt-foundation-focus-width) tmt:disabled:pointer-events-none tmt:disabled:opacity-50 tmt:[&_svg]:pointer-events-none tmt:[&_svg]:shrink-0 tmt:[&_svg:not([class*='size-'])]:size-4",
        'tmt:group-data-[variant=line]/tabs-list:bg-transparent tmt:group-data-[variant=line]/tabs-list:data-[state=active]:bg-transparent',
        'tmt:data-[state=active]:bg-background tmt:data-[state=active]:text-foreground',
        'tmt:after:absolute tmt:after:bg-foreground tmt:after:opacity-0 tmt:group-data-[orientation=horizontal]/tabs:after:inset-x-0 tmt:group-data-[orientation=horizontal]/tabs:after:-bottom-1 tmt:group-data-[orientation=horizontal]/tabs:after:h-(--tmt-foundation-focus-width) tmt:group-data-[orientation=vertical]/tabs:after:inset-y-0 tmt:group-data-[orientation=vertical]/tabs:after:-right-1 tmt:group-data-[orientation=vertical]/tabs:after:w-(--tmt-foundation-focus-width) tmt:group-data-[variant=line]/tabs-list:data-[state=active]:after:opacity-100',
        className,
      )}
      {...props}
    />
  );
}

function TabsContent({ className, ...props }: React.ComponentProps<typeof TabsPrimitive.Content>) {
  return (
    <TabsPrimitive.Content
      data-slot="tabs-content"
      className={cn('tmt:flex-1', className)}
      {...props}
    />
  );
}

export { Tabs, TabsList, TabsTrigger, TabsContent, tabsListVariants };
