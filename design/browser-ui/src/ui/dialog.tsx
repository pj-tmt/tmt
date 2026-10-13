// Adapted from pinned shadcn/ui (MIT); see provenance.json and LICENSE.shadcn.
import type { ComponentProps } from 'react';
import { Dialog as Primitive } from 'radix-ui';
import { XIcon } from 'lucide-react';
import { Button } from './button.js';
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

export function Dialog({
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
export function DialogTrigger(props: ComponentProps<typeof Primitive.Trigger>) {
  return <Primitive.Trigger data-slot="dialog-trigger" {...props} />;
}
export function DialogClose(props: ComponentProps<typeof Primitive.Close>) {
  return <Primitive.Close data-slot="dialog-close" {...props} />;
}
export function DialogPortal(props: Omit<ComponentProps<typeof Primitive.Portal>, 'container'>) {
  const container = useUiPortalContainer();
  return container ? <Primitive.Portal {...props} container={container} /> : null;
}
export function DialogOverlay({ className, ...props }: ComponentProps<typeof Primitive.Overlay>) {
  return (
    <Primitive.Overlay
      data-slot="dialog-overlay"
      className={cn(
        'tmt:fixed tmt:inset-0 tmt:z-(--tmt-foundation-z-layer) tmt:bg-(--tmt-foundation-scrim)',
        className,
      )}
      {...props}
    />
  );
}
export function DialogContent({
  className,
  children,
  showCloseButton = true,
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
}: ComponentProps<typeof Primitive.Content> & LayerFocusOptions & { showCloseButton?: boolean }) {
  const controller = useLayerAdmission();
  const focus = useLayerFocus({ initialFocus, returnFocus, focusFallback }, true);
  const surface = useLayerSurface(ref);
  return (
    <DialogPortal>
      <DialogOverlay />
      <Primitive.Content
        data-slot="dialog-content"
        {...props}
        ref={surface.ref}
        className={cn(
          'tmt:fixed tmt:left-[calc(var(--tmt-layer-left,0px)+var(--tmt-layer-width,100vw)/2)] tmt:top-[calc(var(--tmt-layer-top,0px)+var(--tmt-layer-height,100dvh)/2)] tmt:-translate-x-1/2 tmt:-translate-y-1/2 tmt:z-(--tmt-foundation-z-layer) tmt:grid tmt:gap-4 tmt:w-full tmt:max-w-[min(var(--tmt-layer-available-width,100vw),calc(var(--tmt-foundation-spacing-unit)*128))] tmt:max-h-(--tmt-layer-available-height) tmt:overflow-y-auto tmt:overscroll-contain tmt:border-solid tmt:border-border tmt:border-(length:--tmt-foundation-rule-width) tmt:bg-background tmt:text-foreground tmt:p-6 tmt:outline-none',
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
          admitLayerDismiss(
            event,
            controller,
            event.detail.originalEvent.type === 'pointerdown' ? 'scrim' : 'outside',
          );
        }}
      >
        {children}
        {showCloseButton && (
          <DialogClose
            aria-label="Close"
            className="tmt:absolute tmt:top-4 tmt:right-4 tmt:flex tmt:size-6 tmt:items-center tmt:justify-center tmt:bg-transparent tmt:text-foreground tmt:border-0 tmt:outline-ring tmt:focus-visible:outline-solid tmt:focus-visible:outline-(length:--tmt-foundation-focus-width)"
          >
            <XIcon aria-hidden="true" className="tmt:size-4" />
          </DialogClose>
        )}
      </Primitive.Content>
    </DialogPortal>
  );
}
export function DialogHeader({ className, ...props }: ComponentProps<'div'>) {
  return (
    <div
      data-slot="dialog-header"
      className={cn('tmt:flex tmt:flex-col tmt:gap-2', className)}
      {...props}
    />
  );
}
export function DialogFooter({
  className,
  children,
  showCloseButton = false,
  ...props
}: ComponentProps<'div'> & { showCloseButton?: boolean }) {
  return (
    <div
      data-slot="dialog-footer"
      className={cn('tmt:flex tmt:flex-wrap tmt:justify-end tmt:gap-2', className)}
      {...props}
    >
      {children}
      {showCloseButton && (
        <DialogClose asChild>
          <Button variant="outline">Close</Button>
        </DialogClose>
      )}
    </div>
  );
}
export function DialogTitle({ className, ...props }: ComponentProps<typeof Primitive.Title>) {
  return (
    <Primitive.Title
      data-slot="dialog-title"
      data-layer-title=""
      tabIndex={-1}
      className={cn(
        'tmt:font-semibold tmt:outline-ring tmt:focus:outline-solid tmt:focus:outline-(length:--tmt-foundation-focus-width)',
        className,
      )}
      {...props}
    />
  );
}
export function DialogDescription({
  className,
  ...props
}: ComponentProps<typeof Primitive.Description>) {
  return (
    <Primitive.Description
      data-slot="dialog-description"
      className={cn('tmt:text-muted-foreground', className)}
      {...props}
    />
  );
}
