// Adapted from pinned shadcn/ui (MIT); see provenance.json and LICENSE.shadcn.
import type { ComponentProps } from 'react';
import { Select as Primitive } from 'radix-ui';
import { CheckIcon, ChevronDownIcon, ChevronUpIcon } from 'lucide-react';
import { cn, nativeFieldReset } from './utils.js';
import { useUiPortalContainer } from './theme.js';
import { popMotion } from './motion.js';
import { layerCollisionPadding, useLayerSurface } from './layer.js';

export function Select(props: ComponentProps<typeof Primitive.Root>) {
  return <Primitive.Root data-slot="select" {...props} />;
}
export function SelectGroup(props: ComponentProps<typeof Primitive.Group>) {
  return <Primitive.Group data-slot="select-group" {...props} />;
}
export function SelectValue(props: ComponentProps<typeof Primitive.Value>) {
  return <Primitive.Value data-slot="select-value" {...props} />;
}
/** The trigger is a field: the input's frame, a chevron, and the placeholder in muted ink. */
export function SelectTrigger({
  className,
  children,
  ...props
}: ComponentProps<typeof Primitive.Trigger>) {
  return (
    <Primitive.Trigger
      data-slot="select-trigger"
      className={cn(
        nativeFieldReset,
        'tmt:flex tmt:h-9 tmt:w-fit tmt:items-center tmt:justify-between tmt:gap-2 tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:border-input tmt:bg-background tmt:text-foreground tmt:px-3 tmt:py-1 tmt:text-(length:--tmt-foundation-command-size) tmt:whitespace-nowrap tmt:data-[placeholder]:text-muted-foreground tmt:disabled:cursor-not-allowed tmt:disabled:opacity-50',
        'tmt:focus-visible:border-ring tmt:focus-visible:outline-none tmt:aria-invalid:border-destructive',
        'tmt:[&_svg]:pointer-events-none tmt:[&_svg]:shrink-0 tmt:[&_svg]:size-4 tmt:[&_svg]:text-muted-foreground',
        className,
      )}
      {...props}
    >
      {children}
      <Primitive.Icon asChild>
        <ChevronDownIcon aria-hidden="true" />
      </Primitive.Icon>
    </Primitive.Trigger>
  );
}
/** The list opens in the island's own portal container, as a popper under its trigger. */
export function SelectContent({
  className,
  children,
  position = 'popper',
  sideOffset = 4,
  collisionPadding = layerCollisionPadding,
  ref,
  style,
  ...props
}: ComponentProps<typeof Primitive.Content>) {
  const container = useUiPortalContainer();
  // The same visual-viewport fit as the menu and popover layers (#3057).
  const surface = useLayerSurface(ref);
  if (!container) return null;
  return (
    <Primitive.Portal container={container}>
      <Primitive.Content
        data-slot="select-content"
        ref={surface.ref}
        position={position}
        sideOffset={sideOffset}
        collisionPadding={collisionPadding}
        style={{ ...style, ...surface.viewportStyle }}
        className={cn(
          'tmt:box-border tmt:font-body tmt:text-(length:--tmt-foundation-command-size) tmt:relative tmt:z-(--tmt-foundation-z-layer) tmt:min-w-32 tmt:max-w-(--tmt-layer-available-width) tmt:max-h-[min(var(--radix-select-content-available-height),var(--tmt-layer-available-height))] tmt:overflow-x-hidden tmt:overflow-y-auto tmt:border-solid tmt:border-layer-boundary tmt:border-(length:--tmt-foundation-rule-width) tmt:bg-popover tmt:text-popover-foreground',
          'tmt:rounded-control tmt:shadow-overlay',
          popMotion,
          className,
        )}
        {...props}
      >
        <Primitive.ScrollUpButton className="tmt:flex tmt:cursor-default tmt:items-center tmt:justify-center tmt:py-1">
          <ChevronUpIcon aria-hidden="true" className="tmt:size-4" />
        </Primitive.ScrollUpButton>
        <Primitive.Viewport className="tmt:p-1 tmt:w-full tmt:min-w-(--radix-select-trigger-width)">
          {children}
        </Primitive.Viewport>
        <Primitive.ScrollDownButton className="tmt:flex tmt:cursor-default tmt:items-center tmt:justify-center tmt:py-1">
          <ChevronDownIcon aria-hidden="true" className="tmt:size-4" />
        </Primitive.ScrollDownButton>
      </Primitive.Content>
    </Primitive.Portal>
  );
}
export function SelectLabel({ className, ...props }: ComponentProps<typeof Primitive.Label>) {
  return (
    <Primitive.Label
      data-slot="select-label"
      className={cn(
        'tmt:px-2 tmt:py-1.5 tmt:text-(length:--tmt-foundation-caption-size) tmt:text-muted-foreground',
        className,
      )}
      {...props}
    />
  );
}
export function SelectItem({
  className,
  children,
  ...props
}: ComponentProps<typeof Primitive.Item>) {
  return (
    <Primitive.Item
      data-slot="select-item"
      className={cn(
        'tmt:relative tmt:flex tmt:w-full tmt:cursor-default tmt:items-center tmt:gap-2 tmt:py-1.5 tmt:pr-8 tmt:pl-2 tmt:outline-ring tmt:focus:outline-solid tmt:focus:outline-(length:--tmt-foundation-focus-width) tmt:select-none tmt:focus:bg-accent tmt:focus:text-accent-foreground tmt:data-[disabled]:pointer-events-none tmt:data-[disabled]:text-muted-foreground',
        className,
      )}
      {...props}
    >
      <span className="tmt:absolute tmt:right-2 tmt:flex tmt:size-3.5 tmt:items-center tmt:justify-center">
        <Primitive.ItemIndicator>
          <CheckIcon aria-hidden="true" className="tmt:size-4" />
        </Primitive.ItemIndicator>
      </span>
      <Primitive.ItemText>{children}</Primitive.ItemText>
    </Primitive.Item>
  );
}
export function SelectSeparator({
  className,
  ...props
}: ComponentProps<typeof Primitive.Separator>) {
  return (
    <Primitive.Separator
      data-slot="select-separator"
      className={cn('tmt:pointer-events-none tmt:-mx-1 tmt:my-1 tmt:h-px tmt:bg-border', className)}
      {...props}
    />
  );
}
