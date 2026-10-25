// The Radix-based overlays of the UI layer (`@tmt/browser-ui/ui`) as real pages in frames: they
// share the token roles, the one overlay shadow and the motion tokens with the Browser* overlays.
import { useState } from 'react';
import {
  Button,
  Combobox,
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
  Input,
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
  UiTheme,
  useLayerController,
  type SelectionOption,
} from '../src/ui/index.js';
import { FramePair, Section } from './kit.js';

const projects: SelectionOption[] = [
  { id: 'release', label: 'Release planning', description: 'Colab' },
  { id: 'digest', label: 'Digest rollout', description: 'Remote' },
  { id: 'brand', label: 'Brand refresh' },
  { id: 'archive', label: 'Archived pages', disabled: true },
];

/** An overlay shown open on load. A frame that nothing has focused yet reports the loss of focus
 * as a dismissal; that is not the reader closing it, so the close is held until the frame has focus. */
function useShownOpen(initial: boolean) {
  const [open, setOpen] = useState(initial);
  return {
    open,
    onOpenChange: (next: boolean) => {
      if (next || document.hasFocus()) setOpen(next);
    },
  };
}

function ComboboxDemo({ state }: { state: string }) {
  const shown = useShownOpen(state !== 'closed' && state !== 'disabled' && state !== 'busy');
  const [query, setQuery] = useState(state === 'filtered' ? 'rel' : state === 'empty' ? 'zzz' : '');
  const [value, setValue] = useState<string | null>(null);
  const controller = useLayerController(shown);
  const options = projects.filter((project) =>
    project.label.toLowerCase().includes(query.toLowerCase()),
  );
  const blocked =
    state === 'disabled' ? 'Unavailable: offline' : state === 'busy' ? 'Loading…' : '';
  return (
    <Combobox
      controller={controller}
      label="Project"
      placeholder="Find a project"
      query={query}
      onQueryChange={(next) => {
        setQuery(next);
        controller.requestOpen();
      }}
      options={options}
      value={value}
      onSelect={(option) => {
        setValue(option.id);
        setQuery(option.label);
        controller.requestClose('programmatic');
      }}
      emptyLabel="No project matches"
      disabled={state === 'disabled'}
      busy={state === 'busy'}
      stateLabel={blocked || undefined}
    />
  );
}

function Demo({ kind, state }: { kind: string; state: string }) {
  const open = state !== 'closed';
  const shown = useShownOpen(open);
  switch (kind) {
    case 'dialog':
      return (
        <Dialog defaultOpen={open}>
          <DialogTrigger asChild>
            <Button type="button" variant="outline">
              Open dialog
            </Button>
          </DialogTrigger>
          <DialogContent initialFocus={() => document.getElementById('ui-dialog-project')}>
            <DialogHeader>
              <DialogTitle>Move to project</DialogTitle>
              <DialogDescription>Pick where this page lives.</DialogDescription>
            </DialogHeader>
            <Input id="ui-dialog-project" aria-label="Project" defaultValue="Release planning" />
            <DialogFooter>
              <DialogClose asChild>
                <Button type="button" variant="outline">
                  Cancel
                </Button>
              </DialogClose>
              <DialogClose asChild>
                <Button type="button">Move</Button>
              </DialogClose>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      );
    case 'confirm':
      return (
        <Dialog defaultOpen={open}>
          <DialogTrigger asChild>
            <Button type="button" variant="destructive">
              Delete page
            </Button>
          </DialogTrigger>
          <DialogContent
            showCloseButton={false}
            initialFocus={() => document.getElementById('ui-confirm-keep')}
          >
            <DialogHeader>
              <DialogTitle>Delete this page?</DialogTitle>
              <DialogDescription>Its comments go with it. This cannot be undone.</DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <DialogClose asChild>
                <Button id="ui-confirm-keep" type="button" variant="outline">
                  Keep it
                </Button>
              </DialogClose>
              <DialogClose asChild>
                <Button type="button" variant="destructive">
                  Delete
                </Button>
              </DialogClose>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      );
    case 'sheet':
    case 'sheet-bottom':
      return (
        <Sheet defaultOpen={open}>
          <SheetTrigger asChild>
            <Button type="button" variant="outline">
              Open drawer
            </Button>
          </SheetTrigger>
          <SheetContent
            side={kind === 'sheet' ? 'right' : 'bottom'}
            initialFocus={() => document.getElementById('ui-sheet-title')}
          >
            <SheetHeader>
              <SheetTitle>Page settings</SheetTitle>
              <SheetDescription>Drafts stay here.</SheetDescription>
            </SheetHeader>
            <div style={{ padding: '0 16px' }}>
              <Input id="ui-sheet-title" aria-label="Title" defaultValue="Release plan" />
            </div>
            <SheetFooter>
              <SheetClose asChild>
                <Button type="button" variant="outline">
                  Cancel
                </Button>
              </SheetClose>
              <SheetClose asChild>
                <Button type="button">Save</Button>
              </SheetClose>
            </SheetFooter>
          </SheetContent>
        </Sheet>
      );
    case 'menu':
      return (
        <DropdownMenu {...shown}>
          <DropdownMenuTrigger asChild>
            <Button type="button" variant="outline">
              Page actions
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start">
            <DropdownMenuLabel>Applies to this page</DropdownMenuLabel>
            <DropdownMenuItem>
              Rename <DropdownMenuShortcut>R</DropdownMenuShortcut>
            </DropdownMenuItem>
            <DropdownMenuItem disabled>Move (needs an owner)</DropdownMenuItem>
            <DropdownMenuCheckboxItem checked>Show comments</DropdownMenuCheckboxItem>
            <DropdownMenuSeparator />
            <DropdownMenuRadioGroup value="updated">
              <DropdownMenuRadioItem value="updated">Sort: Updated</DropdownMenuRadioItem>
              <DropdownMenuRadioItem value="title">Sort: Title</DropdownMenuRadioItem>
            </DropdownMenuRadioGroup>
            <DropdownMenuSeparator />
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>Share</DropdownMenuSubTrigger>
              <DropdownMenuSubContent>
                <DropdownMenuItem>Copy link</DropdownMenuItem>
                <DropdownMenuItem>Invite</DropdownMenuItem>
              </DropdownMenuSubContent>
            </DropdownMenuSub>
            <DropdownMenuItem variant="destructive">Delete</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      );
    case 'popover':
      return (
        <Popover {...shown}>
          <PopoverTrigger asChild>
            <Button type="button" variant="outline">
              Details
            </Button>
          </PopoverTrigger>
          <PopoverContent align="start">
            <PopoverHeader>
              <PopoverTitle>Release plan</PopoverTitle>
              <PopoverDescription>Updated 2 minutes ago by core-ops.</PopoverDescription>
            </PopoverHeader>
            <Input aria-label="Note" placeholder="Add a note" />
          </PopoverContent>
        </Popover>
      );
    case 'tooltip':
      return (
        <TooltipProvider>
          <Tooltip {...shown}>
            <TooltipTrigger asChild>
              <Button type="button" variant="outline">
                Copy link
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom">Copy the page link</TooltipContent>
          </Tooltip>
        </TooltipProvider>
      );
    case 'combobox':
      return <ComboboxDemo state={state} />;
    default:
      return <p>Unknown overlay {kind}</p>;
  }
}

/** One overlay of the UI layer on a page of its own, opened on load; `state=closed` leaves the
 * trigger alone so the enter motion can be seen. */
export function UiOverlayFrame({ params }: { params: URLSearchParams }) {
  const theme = document.documentElement.dataset.theme === 'light' ? 'light' : 'dark';
  return (
    <UiTheme theme={theme}>
      <div style={{ padding: 16 }}>
        <Demo kind={params.get('kind') ?? ''} state={params.get('state') ?? 'open'} />
      </div>
    </UiTheme>
  );
}

const wide = { width: 1080 } as const;
const phone = { width: 390 } as const;

export function UiOverlaysSection() {
  return (
    <Section id="ui-overlays">
      <FramePair
        title="Dialog · open"
        note="Escape, Cancel or the close button leaves with the exit motion; the trigger reopens it."
        frame="ui-overlay"
        params={{ kind: 'dialog' }}
        height={420}
        {...wide}
      />
      <FramePair
        title="Dialog · phone"
        frame="ui-overlay"
        params={{ kind: 'dialog' }}
        height={520}
        {...phone}
      />
      <FramePair
        title="Dialog · destructive confirm"
        note="A destructive action uses its own red focus and red hover tint."
        frame="ui-overlay"
        params={{ kind: 'confirm' }}
        height={360}
        {...phone}
      />
      <FramePair
        title="Drawer · from the right edge"
        frame="ui-overlay"
        params={{ kind: 'sheet' }}
        height={420}
        {...wide}
      />
      <FramePair
        title="Drawer · phone, from the bottom"
        frame="ui-overlay"
        params={{ kind: 'sheet-bottom' }}
        height={520}
        {...phone}
      />
      <FramePair
        title="Menu · all row kinds"
        note="Checked, radio, disabled, sub-menu and destructive rows; arrow keys move, Escape closes."
        frame="ui-overlay"
        params={{ kind: 'menu' }}
        height={440}
        width={560}
      />
      <FramePair
        title="Popover"
        frame="ui-overlay"
        params={{ kind: 'popover' }}
        height={300}
        width={560}
      />
      <FramePair
        title="Tooltip"
        frame="ui-overlay"
        params={{ kind: 'tooltip' }}
        height={140}
        width={560}
      />
      <FramePair
        title="Autocomplete · results"
        note="Arrow keys move, Enter picks, Escape closes."
        frame="ui-overlay"
        params={{ kind: 'combobox' }}
        height={360}
        width={560}
      />
      <FramePair
        title="Autocomplete · filtered"
        frame="ui-overlay"
        params={{ kind: 'combobox', state: 'filtered' }}
        height={260}
        width={560}
      />
      <FramePair
        title="Autocomplete · empty"
        frame="ui-overlay"
        params={{ kind: 'combobox', state: 'empty' }}
        height={200}
        width={560}
      />
      <FramePair
        title="Autocomplete · loading"
        frame="ui-overlay"
        params={{ kind: 'combobox', state: 'busy' }}
        height={160}
        width={560}
      />
      <FramePair
        title="Autocomplete · disabled"
        frame="ui-overlay"
        params={{ kind: 'combobox', state: 'disabled' }}
        height={160}
        width={560}
      />
    </Section>
  );
}
