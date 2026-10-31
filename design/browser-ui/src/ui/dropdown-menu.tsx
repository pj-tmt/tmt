// Adapted from pinned shadcn/ui (MIT); see provenance.json and LICENSE.shadcn.
import type { ComponentProps } from 'react';
import { DropdownMenu as Primitive } from 'radix-ui';
import { CheckIcon, ChevronRightIcon, CircleIcon } from 'lucide-react';
import { listItem } from './item.js';
import { cn } from './utils.js';
import { useUiPortalContainer } from './theme.js';
import {
  LayerAdmission,
  admitLayerDismiss,
  layerCollisionPadding,
  useLayerAdmission,
  useLayerFocus,
  useLayerSurface,
  type LayerController,
  type LayerFocusOptions,
} from './layer.js';
import { popMotion } from './motion.js';

export function DropdownMenu({
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
export function DropdownMenuPortal(
  props: Omit<ComponentProps<typeof Primitive.Portal>, 'container'>,
) {
  const container = useUiPortalContainer();
  return container ? <Primitive.Portal {...props} container={container} /> : null;
}
export function DropdownMenuTrigger(props: ComponentProps<typeof Primitive.Trigger>) {
  return <Primitive.Trigger data-slot="dropdown-menu-trigger" {...props} />;
}
export function DropdownMenuContent({
  className,
  sideOffset = 4,
  collisionPadding = layerCollisionPadding,
  returnFocus,
  focusFallback,
  onCloseAutoFocus,
  onEscapeKeyDown,
  onInteractOutside,
  onKeyDown,
  ref,
  style,
  ...props
}: ComponentProps<typeof Primitive.Content> & Omit<LayerFocusOptions, 'initialFocus'>) {
  const controller = useLayerAdmission();
  const focus = useLayerFocus({ returnFocus, focusFallback });
  // Radix owns the menu trigger, including pointer opens that leave a previously
  // focused unrelated control active until entry. Only explicit host targets
  // override its normal return.
  const surface = useLayerSurface(ref);
  return (
    <DropdownMenuPortal>
      <Primitive.Content
        data-slot="dropdown-menu-content"
        {...props}
        ref={surface.ref}
        sideOffset={sideOffset}
        collisionPadding={collisionPadding}
        className={cn(
          'tmt:box-border tmt:font-body tmt:text-(length:--tmt-foundation-command-size) tmt:z-(--tmt-foundation-z-layer) tmt:min-w-32 tmt:max-w-(--tmt-layer-available-width) tmt:max-h-[min(var(--radix-dropdown-menu-content-available-height),var(--tmt-layer-available-height))] tmt:overflow-x-hidden tmt:overflow-y-auto tmt:overscroll-contain tmt:border-solid tmt:border-layer-boundary tmt:border-(length:--tmt-foundation-rule-width) tmt:bg-popover tmt:text-popover-foreground tmt:p-1 tmt:outline-none',
          'tmt:rounded-control tmt:shadow-overlay',
          popMotion,
          className,
        )}
        style={{ ...style, ...surface.viewportStyle }}
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
          if (!event.defaultPrevented && event.detail.originalEvent.type === 'pointerdown')
            event.detail.originalEvent.preventDefault();
          admitLayerDismiss(event, controller, 'outside');
        }}
        onKeyDown={(event) => {
          onKeyDown?.(event);
          if (!event.defaultPrevented && event.key === 'Tab' && controller) {
            event.preventDefault();
            controller.requestClose('tab', event.nativeEvent);
          }
        }}
      />
    </DropdownMenuPortal>
  );
}
export function DropdownMenuGroup(props: ComponentProps<typeof Primitive.Group>) {
  return <Primitive.Group data-slot="dropdown-menu-group" {...props} />;
}
const itemClasses = listItem;
export function DropdownMenuItem({
  className,
  inset,
  variant = 'default',
  ...props
}: ComponentProps<typeof Primitive.Item> & {
  inset?: boolean;
  variant?: 'default' | 'destructive';
}) {
  return (
    <Primitive.Item
      data-slot="dropdown-menu-item"
      data-inset={inset}
      data-variant={variant}
      className={cn(
        itemClasses,
        inset && 'tmt:pl-8',
        variant === 'destructive' && 'tmt:text-destructive',
        className,
      )}
      {...props}
    />
  );
}
export function DropdownMenuCheckboxItem({
  className,
  children,
  checked,
  ...props
}: ComponentProps<typeof Primitive.CheckboxItem>) {
  return (
    <Primitive.CheckboxItem
      data-slot="dropdown-menu-checkbox-item"
      className={cn(itemClasses, 'tmt:pl-8', className)}
      checked={checked}
      {...props}
    >
      <span className="tmt:pointer-events-none tmt:absolute tmt:left-2 tmt:flex tmt:items-center tmt:justify-center">
        <Primitive.ItemIndicator>
          <CheckIcon aria-hidden="true" className="tmt:size-4" />
        </Primitive.ItemIndicator>
      </span>
      {children}
    </Primitive.CheckboxItem>
  );
}
export function DropdownMenuRadioGroup(props: ComponentProps<typeof Primitive.RadioGroup>) {
  return <Primitive.RadioGroup data-slot="dropdown-menu-radio-group" {...props} />;
}
export function DropdownMenuRadioItem({
  className,
  children,
  ...props
}: ComponentProps<typeof Primitive.RadioItem>) {
  return (
    <Primitive.RadioItem
      data-slot="dropdown-menu-radio-item"
      className={cn(itemClasses, 'tmt:pl-8', className)}
      {...props}
    >
      <span className="tmt:pointer-events-none tmt:absolute tmt:left-2 tmt:flex tmt:items-center tmt:justify-center">
        <Primitive.ItemIndicator>
          <CircleIcon aria-hidden="true" className="tmt:size-2 tmt:fill-current" />
        </Primitive.ItemIndicator>
      </span>
      {children}
    </Primitive.RadioItem>
  );
}
export function DropdownMenuLabel({
  className,
  inset,
  ...props
}: ComponentProps<typeof Primitive.Label> & { inset?: boolean }) {
  return (
    <Primitive.Label
      data-slot="dropdown-menu-label"
      className={cn(
        'tmt:px-(--tmt-foundation-item-padding) tmt:py-1.5 tmt:font-medium',
        inset && 'tmt:pl-8',
        className,
      )}
      {...props}
    />
  );
}
export function DropdownMenuSeparator({
  className,
  ...props
}: ComponentProps<typeof Primitive.Separator>) {
  return (
    <Primitive.Separator
      data-slot="dropdown-menu-separator"
      className={cn(
        'tmt:-mx-1 tmt:my-1 tmt:h-(--tmt-foundation-rule-width) tmt:bg-border',
        className,
      )}
      {...props}
    />
  );
}
export function DropdownMenuShortcut({ className, ...props }: ComponentProps<'span'>) {
  return (
    <span
      data-slot="dropdown-menu-shortcut"
      className={cn('tmt:ml-auto tmt:text-muted-foreground', className)}
      {...props}
    />
  );
}
export function DropdownMenuSub(props: ComponentProps<typeof Primitive.Sub>) {
  return <Primitive.Sub {...props} />;
}
export function DropdownMenuSubTrigger({
  className,
  inset,
  children,
  ...props
}: ComponentProps<typeof Primitive.SubTrigger> & { inset?: boolean }) {
  return (
    <Primitive.SubTrigger
      data-slot="dropdown-menu-sub-trigger"
      className={cn(
        itemClasses,
        'tmt:data-[state=open]:bg-accent tmt:data-[state=open]:text-accent-foreground',
        inset && 'tmt:pl-8',
        className,
      )}
      {...props}
    >
      {children}
      <ChevronRightIcon aria-hidden="true" className="tmt:ml-auto tmt:size-4" />
    </Primitive.SubTrigger>
  );
}
export function DropdownMenuSubContent({
  className,
  collisionPadding = layerCollisionPadding,
  ref,
  style,
  ...props
}: ComponentProps<typeof Primitive.SubContent>) {
  const surface = useLayerSurface(ref);
  return (
    <DropdownMenuPortal>
      <Primitive.SubContent
        data-slot="dropdown-menu-sub-content"
        {...props}
        ref={surface.ref}
        collisionPadding={collisionPadding}
        className={cn(
          'tmt:box-border tmt:font-body tmt:text-(length:--tmt-foundation-command-size) tmt:z-(--tmt-foundation-z-layer) tmt:min-w-32 tmt:max-w-(--tmt-layer-available-width) tmt:max-h-[min(var(--radix-dropdown-menu-content-available-height),var(--tmt-layer-available-height))] tmt:overflow-y-auto tmt:border-solid tmt:border-layer-boundary tmt:border-(length:--tmt-foundation-rule-width) tmt:bg-popover tmt:text-popover-foreground tmt:p-1 tmt:outline-none',
          'tmt:rounded-control tmt:shadow-overlay',
          popMotion,
          className,
        )}
        style={{ ...style, ...surface.viewportStyle }}
      />
    </DropdownMenuPortal>
  );
}
