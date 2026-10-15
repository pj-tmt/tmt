// Adapted from pinned shadcn/ui (MIT); see provenance.json and LICENSE.shadcn.
import { useRef, type ComponentProps } from 'react';
import { Command as Primitive } from 'cmdk';
import { SearchIcon } from 'lucide-react';
import { cn } from './utils.js';
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from './dialog.js';

export function Command({ className, ...props }: ComponentProps<typeof Primitive>) {
  return (
    <Primitive
      data-slot="command"
      className={cn(
        'tmt:flex tmt:w-full tmt:min-w-0 tmt:flex-col tmt:font-body tmt:bg-popover tmt:text-popover-foreground',
        className,
      )}
      {...props}
    />
  );
}
export function CommandDialog({
  title = 'Command palette',
  description = 'Search for a command.',
  children,
  className,
  showCloseButton = true,
  ...props
}: ComponentProps<typeof Dialog> & {
  title?: string;
  description?: string;
  className?: string;
  showCloseButton?: boolean;
}) {
  const content = useRef<HTMLDivElement>(null);
  return (
    <Dialog {...props}>
      <DialogContent
        ref={content}
        initialFocus={() =>
          content.current?.querySelector<HTMLInputElement>('[cmdk-input]') ?? null
        }
        className={cn('tmt:overflow-hidden tmt:p-0', className)}
        showCloseButton={showCloseButton}
      >
        <DialogHeader className="tmt:sr-only">
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <Command>{children}</Command>
      </DialogContent>
    </Dialog>
  );
}
export function CommandInput({ className, ...props }: ComponentProps<typeof Primitive.Input>) {
  return (
    <div
      data-slot="command-input-wrapper"
      className="tmt:flex tmt:min-w-0 tmt:items-center tmt:gap-2 tmt:border-0 tmt:border-b tmt:border-solid tmt:border-border tmt:px-3"
    >
      <SearchIcon aria-hidden="true" className="tmt:size-4 tmt:shrink-0" />
      <Primitive.Input
        data-slot="command-input"
        className={cn(
          'tmt:appearance-none tmt:box-border tmt:m-0 tmt:h-10 tmt:w-full tmt:min-w-0 tmt:border-0 tmt:bg-transparent tmt:text-foreground tmt:font-body tmt:text-(length:--tmt-foundation-command-size) tmt:outline-none tmt:focus-visible:outline-solid tmt:focus-visible:outline-ring tmt:focus-visible:outline-(length:--tmt-foundation-focus-width) tmt:placeholder:text-muted-foreground',
          className,
        )}
        {...props}
      />
    </div>
  );
}
export function CommandList({ className, ...props }: ComponentProps<typeof Primitive.List>) {
  return (
    <Primitive.List
      data-slot="command-list"
      className={cn(
        'tmt:min-h-0 tmt:overflow-x-hidden tmt:overflow-y-auto tmt:overscroll-contain tmt:scroll-py-1',
        className,
      )}
      {...props}
    />
  );
}
export function CommandEmpty(props: ComponentProps<typeof Primitive.Empty>) {
  return (
    <Primitive.Empty
      data-slot="command-empty"
      className="tmt:py-6 tmt:text-center tmt:text-(length:--tmt-foundation-command-size)"
      {...props}
    />
  );
}
export function CommandGroup({ className, ...props }: ComponentProps<typeof Primitive.Group>) {
  return (
    <Primitive.Group
      data-slot="command-group"
      className={cn(
        'tmt:overflow-hidden tmt:p-1 tmt:text-foreground tmt:[&_[cmdk-group-heading]]:px-2 tmt:[&_[cmdk-group-heading]]:py-1 tmt:[&_[cmdk-group-heading]]:text-muted-foreground',
        className,
      )}
      {...props}
    />
  );
}
export function CommandSeparator({
  className,
  ...props
}: ComponentProps<typeof Primitive.Separator>) {
  return (
    <Primitive.Separator
      data-slot="command-separator"
      className={cn('tmt:mx-0 tmt:h-px tmt:bg-border', className)}
      {...props}
    />
  );
}
export function CommandItem({ className, ...props }: ComponentProps<typeof Primitive.Item>) {
  return (
    <Primitive.Item
      data-slot="command-item"
      className={cn(
        'tmt:relative tmt:flex tmt:cursor-default tmt:items-center tmt:gap-2 tmt:px-2 tmt:py-2 tmt:text-(length:--tmt-foundation-command-size) tmt:outline-none tmt:select-none tmt:data-[disabled=true]:text-muted-foreground tmt:data-[selected=true]:bg-accent tmt:data-[selected=true]:text-accent-foreground',
        className,
      )}
      {...props}
    />
  );
}
export function CommandShortcut({ className, ...props }: ComponentProps<'span'>) {
  return (
    <span
      data-slot="command-shortcut"
      className={cn(
        'tmt:ml-auto tmt:text-(length:--tmt-foundation-caption-size) tmt:text-muted-foreground',
        className,
      )}
      {...props}
    />
  );
}
