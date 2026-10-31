// Adapted from pinned shadcn/ui (MIT); see provenance.json and LICENSE.shadcn.
import type { ComponentProps } from 'react';
import { Dialog as Primitive } from 'radix-ui';
import { XIcon } from 'lucide-react';
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
import { scrimMotion, slideMotion } from './motion.js';

export function Sheet({
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
export function SheetTrigger(props: ComponentProps<typeof Primitive.Trigger>) {
  return <Primitive.Trigger data-slot="sheet-trigger" {...props} />;
}
export function SheetClose(props: ComponentProps<typeof Primitive.Close>) {
  return <Primitive.Close data-slot="sheet-close" {...props} />;
}
function SheetPortal(props: Omit<ComponentProps<typeof Primitive.Portal>, 'container'>) {
  const container = useUiPortalContainer();
  return container ? <Primitive.Portal {...props} container={container} /> : null;
}
function SheetOverlay() {
  const controller = useLayerAdmission();
  return (
    <Primitive.Overlay
      data-slot="sheet-overlay"
      className={cn(
        'tmt:fixed tmt:inset-0 tmt:z-(--tmt-foundation-z-layer) tmt:bg-(--tmt-foundation-scrim)',
        scrimMotion,
      )}
      onPointerDown={(event) => {
        if (controller) event.preventDefault();
      }}
    />
  );
}
export function SheetContent({
  className,
  children,
  side = 'right',
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
}: ComponentProps<typeof Primitive.Content> &
  LayerFocusOptions & { side?: 'top' | 'right' | 'bottom' | 'left'; showCloseButton?: boolean }) {
  const controller = useLayerAdmission();
  const focus = useLayerFocus({ initialFocus, returnFocus, focusFallback }, true);
  const surface = useLayerSurface(ref);
  return (
    <SheetPortal>
      <SheetOverlay />
      <Primitive.Content
        data-slot="sheet-content"
        data-side={side}
        {...props}
        ref={surface.ref}
        className={cn(
          'tmt:box-border tmt:font-body tmt:text-(length:--tmt-foundation-command-size) tmt:fixed tmt:z-(--tmt-foundation-z-layer) tmt:flex tmt:flex-col tmt:gap-4 tmt:max-w-(--tmt-layer-available-width) tmt:max-h-(--tmt-layer-available-height) tmt:overflow-y-auto tmt:overscroll-contain tmt:bg-background tmt:text-foreground tmt:border-solid tmt:border-layer-boundary tmt:border-(length:--tmt-foundation-rule-width) tmt:outline-none',
          (side === 'left' || side === 'right') &&
            'tmt:top-[calc(var(--tmt-layer-top,0px)+var(--tmt-layer-gutter,0px))] tmt:h-(--tmt-layer-available-height) tmt:w-[calc(var(--tmt-foundation-spacing-unit)*110)]',
          side === 'left' &&
            'tmt:left-[calc(var(--tmt-layer-left,0px)+var(--tmt-layer-gutter,0px))]',
          side === 'right' &&
            'tmt:left-[calc(var(--tmt-layer-left,0px)+var(--tmt-layer-width,100vw)-var(--tmt-layer-gutter,0px))] tmt:-translate-x-full',
          (side === 'top' || side === 'bottom') &&
            'tmt:left-[calc(var(--tmt-layer-left,0px)+var(--tmt-layer-gutter,0px))] tmt:w-(--tmt-layer-available-width)',
          side === 'top' && 'tmt:top-[calc(var(--tmt-layer-top,0px)+var(--tmt-layer-gutter,0px))]',
          side === 'bottom' &&
            'tmt:top-[calc(var(--tmt-layer-top,0px)+var(--tmt-layer-height,100dvh)-var(--tmt-layer-gutter,0px))] tmt:-translate-y-full',
          'tmt:rounded-surface tmt:shadow-elevation-3',
          slideMotion[side],
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
          <SheetClose
            aria-label="Close"
            className="tmt:absolute tmt:top-4 tmt:right-4 tmt:flex tmt:size-6 tmt:items-center tmt:justify-center tmt:bg-transparent tmt:text-foreground tmt:border-0 tmt:outline-ring tmt:focus-visible:outline-solid tmt:focus-visible:outline-(length:--tmt-foundation-focus-width) tmt:focus-visible:-outline-offset-(length:--tmt-foundation-focus-width)"
          >
            <XIcon aria-hidden="true" className="tmt:size-4" />
          </SheetClose>
        )}
      </Primitive.Content>
    </SheetPortal>
  );
}
export function SheetHeader({ className, ...props }: ComponentProps<'div'>) {
  return (
    <div
      data-slot="sheet-header"
      className={cn('tmt:flex tmt:flex-col tmt:gap-2 tmt:p-4', className)}
      {...props}
    />
  );
}
export function SheetFooter({ className, ...props }: ComponentProps<'div'>) {
  return (
    <div
      data-slot="sheet-footer"
      className={cn('tmt:mt-auto tmt:flex tmt:flex-col tmt:gap-2 tmt:p-4', className)}
      {...props}
    />
  );
}
export function SheetTitle({ className, ...props }: ComponentProps<typeof Primitive.Title>) {
  return (
    <Primitive.Title
      data-slot="sheet-title"
      data-layer-title=""
      tabIndex={-1}
      className={cn(
        'tmt:m-0 tmt:text-(length:--tmt-foundation-command-size) tmt:font-(weight:--tmt-foundation-eyebrow-weight) tmt:outline-ring tmt:focus-visible:outline-solid tmt:focus-visible:outline-(length:--tmt-foundation-focus-width)',
        className,
      )}
      {...props}
    />
  );
}
export function SheetDescription({
  className,
  ...props
}: ComponentProps<typeof Primitive.Description>) {
  return (
    <Primitive.Description
      data-slot="sheet-description"
      className={cn('tmt:m-0 tmt:text-muted-foreground', className)}
      {...props}
    />
  );
}
