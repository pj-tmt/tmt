/** The one row of every list-style layer (#5821): Select, the menus and Command. A highlighted row is a
 * neutral fill inset from the list edge with the small radius, never an accent ring (the list would
 * clip it); focus stays visible on the layer's trigger or field. Every list shares one inline padding. */
export const listItem =
  'tmt:box-border tmt:relative tmt:flex tmt:cursor-default tmt:items-center tmt:gap-2 tmt:rounded-small tmt:px-(--tmt-foundation-item-padding) tmt:py-1.5 tmt:outline-none tmt:select-none tmt:data-[highlighted]:bg-accent tmt:data-[highlighted]:text-accent-foreground';
/** Radix marks a disabled row by the attribute's presence; cmdk writes `data-disabled="false"` on every
 * row, so Command states its own `data-[disabled=true]` instead. */
export const radixItemDisabled =
  'tmt:data-[disabled]:pointer-events-none tmt:data-[disabled]:text-muted-foreground';
