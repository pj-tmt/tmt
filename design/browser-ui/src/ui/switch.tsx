// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
'use client';

import * as React from 'react';
import { Switch as SwitchPrimitive } from 'radix-ui';

import { cn } from './utils.js';

function Switch({
  className,
  size = 'default',
  readOnly = false,
  onClick,
  ...props
}: React.ComponentProps<typeof SwitchPrimitive.Root> & {
  size?: 'sm' | 'default';
  readOnly?: boolean;
}) {
  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      data-size={size}
      className={cn(
        'tmt:p-0 tmt:bg-background tmt:text-foreground tmt:appearance-none tmt:[border-radius:var(--tmt-foundation-radius)] tmt:font-body tmt:m-0 tmt:box-border',
        'tmt:peer tmt:group/switch tmt:inline-flex tmt:shrink-0 tmt:items-center tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:border-transparent tmt:focus-visible:border-ring tmt:focus-visible:outline-solid tmt:focus-visible:outline-ring tmt:focus-visible:outline-(length:--tmt-foundation-focus-width) tmt:disabled:cursor-not-allowed tmt:disabled:opacity-50 tmt:data-[size=default]:h-5 tmt:data-[size=default]:w-8 tmt:data-[size=sm]:h-3.5 tmt:data-[size=sm]:w-6 tmt:data-[state=checked]:bg-primary tmt:data-[state=unchecked]:bg-indicator',
        className,
      )}
      {...props}
      aria-readonly={readOnly || undefined}
      onClick={(event) => {
        if (readOnly) event.preventDefault();
        else onClick?.(event);
      }}
    >
      <SwitchPrimitive.Thumb
        data-slot="switch-thumb"
        className={cn(
          'tmt:appearance-none tmt:[border-radius:var(--tmt-foundation-radius)] tmt:font-body tmt:m-0 tmt:box-border',
          'tmt:pointer-events-none tmt:block tmt:bg-background tmt:group-data-[size=default]/switch:size-4 tmt:group-data-[size=sm]/switch:size-3 tmt:data-[state=checked]:translate-x-[calc(100%-var(--tmt-foundation-rule-width)*2)] tmt:data-[state=unchecked]:translate-x-0',
        )}
      />
    </SwitchPrimitive.Root>
  );
}

export { Switch };
