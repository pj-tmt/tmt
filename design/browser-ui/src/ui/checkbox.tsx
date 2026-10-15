// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
'use client';

import * as React from 'react';
import { CheckIcon } from 'lucide-react';
import { Checkbox as CheckboxPrimitive } from 'radix-ui';

import { cn } from './utils.js';

function Checkbox({
  className,
  readOnly = false,
  onClick,
  ...props
}: React.ComponentProps<typeof CheckboxPrimitive.Root> & { readOnly?: boolean }) {
  return (
    <CheckboxPrimitive.Root
      data-slot="checkbox"
      className={cn(
        'tmt:p-0 tmt:bg-background tmt:text-foreground tmt:appearance-none tmt:[border-radius:var(--tmt-foundation-radius)] tmt:font-body tmt:m-0 tmt:box-border',
        'tmt:peer tmt:size-4 tmt:shrink-0 tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:border-input tmt:focus-visible:border-ring tmt:focus-visible:outline-solid tmt:focus-visible:outline-ring tmt:focus-visible:outline-(length:--tmt-foundation-focus-width) tmt:disabled:cursor-not-allowed tmt:disabled:opacity-50 tmt:aria-invalid:border-destructive tmt:data-[state=checked]:border-primary tmt:data-[state=checked]:bg-primary tmt:data-[state=checked]:text-primary-foreground',
        className,
      )}
      {...props}
      aria-readonly={readOnly || undefined}
      onClick={(event) => {
        if (readOnly) event.preventDefault();
        else onClick?.(event);
      }}
    >
      <CheckboxPrimitive.Indicator
        data-slot="checkbox-indicator"
        className="tmt:grid tmt:place-content-center tmt:text-current"
      >
        <CheckIcon className="tmt:size-3.5" />
      </CheckboxPrimitive.Indicator>
    </CheckboxPrimitive.Root>
  );
}

export { Checkbox };
