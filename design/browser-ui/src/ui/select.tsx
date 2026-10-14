// Adapted from pinned shadcn/ui (MIT); see LICENSE.shadcn and provenance.json.
'use client';

import * as React from 'react';
import { CheckIcon, ChevronDownIcon, ChevronUpIcon } from 'lucide-react';
import { Select as SelectPrimitive } from 'radix-ui';

import { cn } from './utils.js';
import { useUiPortalContainer } from './theme.js';

const ReadOnly = React.createContext(false);

function Select({
  readOnly = false,
  value,
  defaultValue,
  onValueChange,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Root> & { readOnly?: boolean }) {
  // Radix keeps its own default-value semantics; read-only admission needs a host value.
  if (readOnly && value === undefined)
    throw new Error('Read-only Select requires a controlled value');
  return (
    <ReadOnly.Provider value={readOnly}>
      <SelectPrimitive.Root
        {...props}
        value={value}
        defaultValue={defaultValue}
        onValueChange={readOnly ? undefined : onValueChange}
      />
    </ReadOnly.Provider>
  );
}

function SelectGroup({ ...props }: React.ComponentProps<typeof SelectPrimitive.Group>) {
  return <SelectPrimitive.Group data-slot="select-group" {...props} />;
}

function SelectValue({ ...props }: React.ComponentProps<typeof SelectPrimitive.Value>) {
  return <SelectPrimitive.Value data-slot="select-value" {...props} />;
}

function SelectTrigger({
  className,
  size = 'default',
  children,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Trigger> & {
  size?: 'sm' | 'default';
}) {
  const readOnly = React.useContext(ReadOnly);
  return (
    <SelectPrimitive.Trigger
      data-slot="select-trigger"
      data-size={size}
      className={cn(
        'tmt:appearance-none tmt:[border-radius:var(--tmt-foundation-radius)] tmt:font-body tmt:m-0 tmt:box-border',
        "tmt:flex tmt:w-fit tmt:items-center tmt:justify-between tmt:gap-2 tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:border-input tmt:bg-background tmt:text-foreground tmt:px-3 tmt:py-2 tmt:text-(length:--tmt-foundation-command-size) tmt:whitespace-nowrap tmt:focus-visible:border-ring tmt:focus-visible:outline-solid tmt:focus-visible:outline-ring tmt:focus-visible:outline-(length:--tmt-foundation-focus-width) tmt:disabled:cursor-not-allowed tmt:disabled:opacity-50 tmt:aria-invalid:border-destructive tmt:data-[placeholder]:text-muted-foreground tmt:data-[size=default]:h-9 tmt:data-[size=sm]:h-8 tmt:*:data-[slot=select-value]:line-clamp-1 tmt:*:data-[slot=select-value]:flex tmt:*:data-[slot=select-value]:items-center tmt:*:data-[slot=select-value]:gap-2 tmt:[&_svg]:pointer-events-none tmt:[&_svg]:shrink-0 tmt:[&_svg:not([class*='size-'])]:size-4 tmt:[&_svg:not([class*='text-'])]:text-muted-foreground",
        className,
      )}
      {...props}
      aria-readonly={readOnly || undefined}
    >
      {children}
      <SelectPrimitive.Icon asChild>
        <ChevronDownIcon className="tmt:size-4 tmt:opacity-50" />
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  );
}

function SelectContent({
  className,
  children,
  position = 'popper',
  align = 'center',
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Content>) {
  const container = useUiPortalContainer();
  if (!container) return null;
  return (
    <SelectPrimitive.Portal container={container}>
      <SelectPrimitive.Content
        data-slot="select-content"
        className={cn(
          'tmt:appearance-none tmt:[border-radius:var(--tmt-foundation-radius)] tmt:font-body tmt:m-0 tmt:box-border',
          'tmt:relative tmt:z-(--tmt-foundation-z-layer) tmt:max-h-(--radix-select-content-available-height) tmt:max-w-(--radix-select-content-available-width) tmt:min-w-32 tmt:origin-(--radix-select-content-transform-origin) tmt:overflow-x-hidden tmt:overflow-y-auto tmt:border-solid tmt:border-(length:--tmt-foundation-rule-width) tmt:border-border tmt:bg-popover tmt:text-popover-foreground',
          position === 'popper' &&
            'tmt:data-[side=bottom]:translate-y-1 tmt:data-[side=left]:-translate-x-1 tmt:data-[side=right]:translate-x-1 tmt:data-[side=top]:-translate-y-1',
          className,
        )}
        position={position}
        align={align}
        {...props}
      >
        <SelectScrollUpButton />
        <SelectPrimitive.Viewport
          className={cn(
            'tmt:appearance-none tmt:[border-radius:var(--tmt-foundation-radius)] tmt:font-body tmt:m-0 tmt:box-border',
            'tmt:p-1',
            position === 'popper' &&
              'tmt:h-[var(--radix-select-trigger-height)] tmt:w-full tmt:min-w-[var(--radix-select-trigger-width)] tmt:scroll-my-1',
          )}
        >
          {children}
        </SelectPrimitive.Viewport>
        <SelectScrollDownButton />
      </SelectPrimitive.Content>
    </SelectPrimitive.Portal>
  );
}

function SelectLabel({ className, ...props }: React.ComponentProps<typeof SelectPrimitive.Label>) {
  return (
    <SelectPrimitive.Label
      data-slot="select-label"
      className={cn(
        'tmt:px-2 tmt:py-1.5 tmt:text-(length:--tmt-foundation-caption-size) tmt:text-muted-foreground',
        className,
      )}
      {...props}
    />
  );
}

function SelectItem({
  className,
  children,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Item>) {
  return (
    <SelectPrimitive.Item
      data-slot="select-item"
      className={cn(
        'tmt:appearance-none tmt:[border-radius:var(--tmt-foundation-radius)] tmt:font-body tmt:m-0 tmt:box-border',
        "tmt:relative tmt:flex tmt:w-full tmt:cursor-default tmt:items-center tmt:gap-2 tmt:py-1.5 tmt:pr-8 tmt:pl-2 tmt:text-(length:--tmt-foundation-command-size) tmt:select-none tmt:focus:bg-accent tmt:focus:text-accent-foreground tmt:focus-visible:outline-solid tmt:focus-visible:outline-ring tmt:focus-visible:outline-(length:--tmt-foundation-focus-width) tmt:data-[disabled]:pointer-events-none tmt:data-[disabled]:opacity-50 tmt:[&_svg]:pointer-events-none tmt:[&_svg]:shrink-0 tmt:[&_svg:not([class*='size-'])]:size-4 tmt:[&_svg:not([class*='text-'])]:text-muted-foreground tmt:*:[span]:last:flex tmt:*:[span]:last:items-center tmt:*:[span]:last:gap-2",
        className,
      )}
      {...props}
    >
      <span
        data-slot="select-item-indicator"
        className="tmt:absolute tmt:right-2 tmt:flex tmt:size-3.5 tmt:items-center tmt:justify-center"
      >
        <SelectPrimitive.ItemIndicator>
          <CheckIcon className="tmt:size-4" />
        </SelectPrimitive.ItemIndicator>
      </span>
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
    </SelectPrimitive.Item>
  );
}

function SelectSeparator({
  className,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Separator>) {
  return (
    <SelectPrimitive.Separator
      data-slot="select-separator"
      className={cn(
        'tmt:pointer-events-none tmt:-mx-1 tmt:my-1 tmt:h-(--tmt-foundation-rule-width) tmt:bg-border',
        className,
      )}
      {...props}
    />
  );
}

function SelectScrollUpButton({
  className,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.ScrollUpButton>) {
  return (
    <SelectPrimitive.ScrollUpButton
      data-slot="select-scroll-up-button"
      className={cn(
        'tmt:appearance-none tmt:[border-radius:var(--tmt-foundation-radius)] tmt:font-body tmt:m-0 tmt:box-border',
        'tmt:flex tmt:cursor-default tmt:items-center tmt:justify-center tmt:py-1',
        className,
      )}
      {...props}
    >
      <ChevronUpIcon className="tmt:size-4" />
    </SelectPrimitive.ScrollUpButton>
  );
}

function SelectScrollDownButton({
  className,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.ScrollDownButton>) {
  return (
    <SelectPrimitive.ScrollDownButton
      data-slot="select-scroll-down-button"
      className={cn(
        'tmt:appearance-none tmt:[border-radius:var(--tmt-foundation-radius)] tmt:font-body tmt:m-0 tmt:box-border',
        'tmt:flex tmt:cursor-default tmt:items-center tmt:justify-center tmt:py-1',
        className,
      )}
      {...props}
    >
      <ChevronDownIcon className="tmt:size-4" />
    </SelectPrimitive.ScrollDownButton>
  );
}

export {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectScrollDownButton,
  SelectScrollUpButton,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
};
