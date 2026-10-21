// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
import * as React from 'react';

import { cn, nativeFieldReset } from './utils.js';

function Textarea({ className, ...props }: React.ComponentProps<'textarea'>) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        nativeFieldReset,
        'tmt:flex tmt:field-sizing-content tmt:min-h-16 tmt:w-full tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:border-input tmt:bg-background tmt:text-foreground tmt:px-3 tmt:py-2 tmt:text-(length:--tmt-foundation-command-size) tmt:placeholder:text-muted-foreground tmt:focus-visible:border-ring tmt:focus-visible:outline-none tmt:disabled:cursor-not-allowed tmt:disabled:opacity-50 tmt:aria-invalid:border-destructive',
        className,
      )}
      {...props}
    />
  );
}

export { Textarea };
