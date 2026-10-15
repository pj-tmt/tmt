// Adapted from pinned shadcn/ui (MIT); see provenance.json and LICENSE.shadcn.
import type { ComponentProps } from 'react';
import { Popover as Primitive } from 'radix-ui';
import { cn } from './utils.js';
import { useUiPortalContainer } from './theme.js';
import {
  LayerAdmission,
  admitLayerDismiss,
  useLayerAdmission,
  useLayerFocus,
  useLayerSurface,
  type LayerController,
  type LayerFocusOptions,
} from './layer.js';

export function Popover({
  controller,
  ...props
}: ComponentProps<typeof Primitive.Root> & { controller?: LayerController }) {
  return (
    <LayerAdmission.Provider value={controller}>
      <Primitive.Root
        {...props}
        {...(controller
          ? {
              open: controller.open,
              onOpenChange: (open: boolean) =>
                open ? controller.requestOpen() : controller.requestClose('close'),
            }
          : {})}
      />
    </LayerAdmission.Provider>
  );
}
export function PopoverTrigger(props: ComponentProps<typeof Primitive.Trigger>) {
  return <Primitive.Trigger data-slot="popover-trigger" {...props} />;
}
export function PopoverAnchor(props: ComponentProps<typeof Primitive.Anchor>) {
  return <Primitive.Anchor data-slot="popover-anchor" {...props} />;
}
export function PopoverContent({
  className,
  align = 'center',
  sideOffset = 4,
  initialFocus,
  returnFocus,
  focusFallback,
  onOpenAutoFocus,
  onCloseAutoFocus,
  onEscapeKeyDown,
  onInteractOutside,
  ref,
  style,
  ...props
}: ComponentProps<typeof Primitive.Content> & LayerFocusOptions) {
  const container = useUiPortalContainer();
  const controller = useLayerAdmission();
  const focus = useLayerFocus({ initialFocus, returnFocus, focusFallback });
  const surface = useLayerSurface(ref);
  return container ? (
    <Primitive.Portal container={container}>
      <Primitive.Content
        data-slot="popover-content"
        {...props}
        ref={surface.ref}
        align={align}
        sideOffset={sideOffset}
        className={cn(
          'tmt:box-border tmt:font-body tmt:text-(length:--tmt-foundation-command-size) tmt:z-(--tmt-foundation-z-layer) tmt:w-72 tmt:max-w-(--tmt-layer-available-width) tmt:max-h-[min(var(--radix-popover-content-available-height),var(--tmt-layer-available-height))] tmt:overflow-y-auto tmt:overscroll-contain tmt:border-solid tmt:border-border tmt:border-(length:--tmt-foundation-rule-width) tmt:bg-popover tmt:text-popover-foreground tmt:p-4 tmt:outline-none',
          className,
        )}
        style={{ ...style, ...surface.viewportStyle }}
        onOpenAutoFocus={(event) => {
          onOpenAutoFocus?.(event);
          focus.onOpenAutoFocus(event);
        }}
        onCloseAutoFocus={(event) => {
          onCloseAutoFocus?.(event);
          focus.onCloseAutoFocus(event);
        }}
        onEscapeKeyDown={(event) => {
          onEscapeKeyDown?.(event);
          admitLayerDismiss(event, controller, 'escape');
        }}
        onInteractOutside={(event) => {
          onInteractOutside?.(event);
          admitLayerDismiss(event, controller, 'outside');
        }}
      />
    </Primitive.Portal>
  ) : null;
}
export function PopoverHeader({ className, ...props }: ComponentProps<'div'>) {
  return (
    <div
      data-slot="popover-header"
      className={cn('tmt:flex tmt:flex-col tmt:gap-1', className)}
      {...props}
    />
  );
}
export function PopoverTitle({ className, ...props }: ComponentProps<'h2'>) {
  return (
    <h2
      data-slot="popover-title"
      className={cn(
        'tmt:m-0 tmt:text-(length:--tmt-foundation-command-size) tmt:font-(weight:--tmt-foundation-eyebrow-weight)',
        className,
      )}
      {...props}
    />
  );
}
export function PopoverDescription({ className, ...props }: ComponentProps<'p'>) {
  return (
    <p
      data-slot="popover-description"
      className={cn('tmt:m-0 tmt:text-muted-foreground', className)}
      {...props}
    />
  );
}
