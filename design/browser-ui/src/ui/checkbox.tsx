// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
'use client';

import * as React from 'react';
import { CheckIcon } from 'lucide-react';
import { Checkbox as CheckboxPrimitive } from 'radix-ui';

import { assignRefs, cn } from './utils.js';

function Checkbox({
  className,
  readOnly = false,
  onClick,
  form,
  ref,
  ...props
}: React.ComponentProps<typeof CheckboxPrimitive.Root> & { readOnly?: boolean }) {
  // Radix gives `form` to the hidden input only and resets on the trigger's own form owner,
  // which is none for a control outside its form. Naming the form on the trigger as well lets
  // the browser resolve that owner, as Switch does by id (#3388).
  const associate = React.useCallback(
    (node: HTMLButtonElement | null) => {
      if (!node) return;
      const release = assignRefs(node, ref);
      if (form) node.setAttribute('form', form);
      return () => {
        if (form) node.removeAttribute('form');
        release();
      };
    },
    [form, ref],
  );
  return (
    <CheckboxPrimitive.Root
      ref={associate}
      form={form}
      data-slot="checkbox"
      className={cn(
        'tmt:p-0 tmt:bg-background tmt:text-foreground tmt:appearance-none tmt:[border-radius:var(--tmt-foundation-radius)] tmt:font-body tmt:m-0 tmt:box-border',
        'tmt:peer tmt:size-4 tmt:shrink-0 tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:border-indicator tmt:focus-visible:border-ring tmt:focus-visible:outline-solid tmt:focus-visible:outline-ring tmt:focus-visible:outline-(length:--tmt-foundation-focus-width) tmt:disabled:cursor-not-allowed tmt:disabled:opacity-50 tmt:aria-invalid:border-destructive tmt:data-[state=checked]:border-primary tmt:data-[state=checked]:bg-primary tmt:data-[state=checked]:text-primary-foreground',
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
