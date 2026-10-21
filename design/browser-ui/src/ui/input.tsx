// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
import * as React from 'react';

import { cn, nativeFieldReset } from './utils.js';

function Input({ className, type, ...props }: React.ComponentProps<'input'>) {
  return (
    <input
      type={type}
      data-slot="input"
      className={cn(
        nativeFieldReset,
        'tmt:h-9 tmt:w-full tmt:min-w-0 tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:border-input tmt:bg-background tmt:text-foreground tmt:px-3 tmt:py-1 tmt:text-(length:--tmt-foundation-command-size) tmt:selection:bg-primary tmt:selection:text-primary-foreground tmt:file:inline-flex tmt:file:h-7 tmt:file:border-0 tmt:file:bg-transparent tmt:file:text-(length:--tmt-foundation-command-size) tmt:file:font-(weight:--tmt-foundation-eyebrow-weight) tmt:file:text-foreground tmt:placeholder:text-muted-foreground tmt:disabled:pointer-events-none tmt:disabled:cursor-not-allowed tmt:disabled:opacity-50',
        'tmt:focus-visible:border-ring tmt:focus-visible:outline-none',
        'tmt:aria-invalid:border-destructive',
        className,
      )}
      {...props}
    />
  );
}

export { Input };
