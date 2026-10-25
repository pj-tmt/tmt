// Adapted from pinned shadcn/ui (MIT); see provenance.json and LICENSE.shadcn.
import type { ComponentProps } from 'react';
import { Popover as Primitive, Portal } from 'radix-ui';
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
import { popMotion } from './motion.js';

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
  outsidePress = 'through',
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
}: ComponentProps<typeof Primitive.Content> &
  LayerFocusOptions & {
    outsidePress?: 'through' | 'dismiss-only';
  }) {
  const container = useUiPortalContainer();
  const controller = useLayerAdmission();
  const focus = useLayerFocus({ initialFocus, returnFocus, focusFallback });
  const surface = useLayerSurface(ref);
  if (outsidePress === 'dismiss-only' && !controller)
    throw new Error('Dismiss-only Popover requires its host layer controller');
  const shield =
    outsidePress === 'dismiss-only' && controller?.open && container ? (
      <Portal.Root asChild container={container}>
        <div
          data-slot="popover-outside-shield"
          aria-hidden="true"
          className="tmt:fixed tmt:inset-0 tmt:z-(--tmt-foundation-z-layer) tmt:bg-transparent"
          onPointerDown={(event) => {
            // Swallow the first press even when the host vetoes closing. The
            // shield, not a deferred outside observer, owns this real target.
            event.preventDefault();
            event.stopPropagation();
            controller.requestClose('outside', event.nativeEvent);
          }}
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
          }}
          onContextMenu={(event) => {
            event.preventDefault();
            event.stopPropagation();
          }}
        />
      </Portal.Root>
    ) : null;
  // The content is the portal's direct child, so its own Presence can play the exit animation.
  return container ? (
    <>
      {shield}
      <Primitive.Portal container={container}>
        <Primitive.Content
          data-slot="popover-content"
          {...props}
          ref={surface.ref}
          align={align}
          sideOffset={sideOffset}
          className={cn(
            'tmt:box-border tmt:font-body tmt:text-(length:--tmt-foundation-command-size) tmt:z-(--tmt-foundation-z-layer) tmt:w-72 tmt:max-w-(--tmt-layer-available-width) tmt:max-h-[min(var(--radix-popover-content-available-height),var(--tmt-layer-available-height))] tmt:overflow-y-auto tmt:overscroll-contain tmt:border-solid tmt:border-layer-boundary tmt:border-(length:--tmt-foundation-rule-width) tmt:bg-popover tmt:text-popover-foreground tmt:p-4 tmt:outline-none',
            'tmt:rounded-control tmt:shadow-overlay',
            popMotion,
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
    </>
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
