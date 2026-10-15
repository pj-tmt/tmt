// Adapted from pinned shadcn/ui (MIT); stock Radix timing/Escape, no busy-veto claim.
import type { ComponentProps } from 'react';
import { Tooltip as Primitive } from 'radix-ui';
import { cn } from './utils.js';
import { useUiPortalContainer } from './theme.js';
import { useLayerSurface } from './layer.js';

export function TooltipProvider({
  delayDuration = 0,
  ...props
}: ComponentProps<typeof Primitive.Provider>) {
  return <Primitive.Provider delayDuration={delayDuration} {...props} />;
}
export function Tooltip(props: ComponentProps<typeof Primitive.Root>) {
  return <Primitive.Root {...props} />;
}
export function TooltipTrigger(props: ComponentProps<typeof Primitive.Trigger>) {
  return <Primitive.Trigger data-slot="tooltip-trigger" {...props} />;
}
export function TooltipContent({
  className,
  sideOffset = 0,
  children,
  ref,
  style,
  ...props
}: ComponentProps<typeof Primitive.Content>) {
  const container = useUiPortalContainer();
  const surface = useLayerSurface(ref);
  return container ? (
    <Primitive.Portal container={container}>
      <Primitive.Content
        data-slot="tooltip-content"
        {...props}
        ref={surface.ref}
        sideOffset={sideOffset}
        className={cn(
          'tmt:box-border tmt:font-body tmt:text-(length:--tmt-foundation-command-size) tmt:z-(--tmt-foundation-z-tooltip) tmt:w-fit tmt:max-w-(--tmt-layer-available-width) tmt:bg-foreground tmt:text-background tmt:px-3 tmt:py-1.5 tmt:text-balance',
          className,
        )}
        style={{ ...style, ...surface.viewportStyle }}
      >
        {children}
        <Primitive.Arrow aria-hidden="true" className="tmt:fill-foreground" />
      </Primitive.Content>
    </Primitive.Portal>
  ) : null;
}
