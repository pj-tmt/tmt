// Adapted from shadcn/ui (MIT); see ./LICENSE.shadcn and ./provenance.json.
import { clsx, type ClassValue } from 'clsx';
import { extendTailwindMerge } from 'tailwind-merge';

const merge = extendTailwindMerge({ prefix: 'tmt' });

/** shadcn clsx/merge helper, configured for this leaf's fixed Tailwind v4 prefix. */
export function cn(...inputs: ClassValue[]): string {
  return merge(clsx(inputs));
}
