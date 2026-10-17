// Adapted from shadcn/ui (MIT); see ./LICENSE.shadcn and ./provenance.json.
import type { Ref } from 'react';
import { clsx, type ClassValue } from 'clsx';
import { extendTailwindMerge } from 'tailwind-merge';

const merge = extendTailwindMerge({ prefix: 'tmt' });

/** shadcn clsx/merge helper, configured for this leaf's fixed Tailwind v4 prefix. */
export function cn(...inputs: ClassValue[]): string {
  return merge(clsx(inputs));
}

/** Bridge local and host refs, preserving React callback-ref cleanup. */
export function assignRefs<T>(node: T | null, ...refs: (Ref<T> | undefined)[]): () => void {
  const cleanups = refs.map((ref) => {
    const cleanup = typeof ref === 'function' ? ref(node) : undefined;
    if (ref && typeof ref !== 'function') ref.current = node;
    return () => {
      if (typeof cleanup === 'function') cleanup();
      else if (typeof ref === 'function') ref(null);
      else if (ref) ref.current = null;
    };
  });
  return () => {
    for (const cleanup of cleanups) cleanup();
  };
}

/** Shared native Input/Textarea reset; keep the complete literal for CSS generation. */
export const nativeFieldReset =
  'tmt:appearance-none tmt:[border-radius:var(--tmt-foundation-radius)] tmt:font-body tmt:m-0 tmt:box-border';
