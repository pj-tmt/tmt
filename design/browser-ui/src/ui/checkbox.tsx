// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
'use client';

import * as React from 'react';
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
      // The shared static drawing (`tmt-ui-checkbox` in static.css) keyed on Radix's data-state;
      // one look for native and Radix checkboxes.
      className={cn('tmt-ui-checkbox', className)}
      {...props}
      aria-readonly={readOnly || undefined}
      onClick={(event) => {
        if (readOnly) event.preventDefault();
        else onClick?.(event);
      }}
    />
  );
}

export { Checkbox };
