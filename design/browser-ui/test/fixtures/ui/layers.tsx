import { StrictMode, useRef, useState } from 'react';
import { Button, Badge } from '../../../src/ui/primitives.js';
import {
  Dialog,
  DialogTrigger,
  DialogContent,
  DialogTitle,
  DialogDescription,
  DialogHeader,
  DialogFooter,
  Sheet,
  SheetTrigger,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSub,
  DropdownMenuSubTrigger,
  DropdownMenuSubContent,
  DropdownMenuCheckboxItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  Popover,
  PopoverTrigger,
  PopoverContent,
  PopoverHeader,
  PopoverTitle,
  PopoverDescription,
  Tooltip,
  TooltipProvider,
  TooltipTrigger,
  TooltipContent,
  useLayerController,
  useMenuLayerHandoff,
  type LayerCloseRequest,
} from '../../../src/ui/layers.js';

function LayerGallery() {
  const fallback = useRef<HTMLButtonElement>(null);
  const menuTrigger = useRef<HTMLButtonElement>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [sheetOpen, setSheetOpen] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [popoverOpen, setPopoverOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const [openerPresent, setOpenerPresent] = useState(true);
  const [mounted, setMounted] = useState(true);
  const [outsideActions, setOutsideActions] = useState(0);
  const [menuActions, setMenuActions] = useState(0);
  const [denied, setDenied] = useState(0);
  const [dialogEntries, setDialogEntries] = useState(0);
  const [sheetEntries, setSheetEntries] = useState(0);
  const [menuCloseFocusEvents, setMenuCloseFocusEvents] = useState(0);
  const [lastClose, setLastClose] = useState('none');
  const [checked, setChecked] = useState(false);
  const [choice, setChoice] = useState('one');
  const [draft, setDraft] = useState('Kept private draft');
  const canClose = (request: LayerCloseRequest) => {
    setLastClose(request.reason);
    if (!busy && !uncertain) return true;
    setDenied((count) => count + 1);
    return false;
  };
  const dialog = useLayerController({
    open: dialogOpen,
    onOpenChange: (open) => {
      setDialogOpen(open);
      if (open) setDialogEntries((n) => n + 1);
    },
    canClose,
  });
  const sheet = useLayerController({
    open: sheetOpen,
    onOpenChange: (open) => {
      setSheetOpen(open);
      if (open) setSheetEntries((n) => n + 1);
    },
    canClose,
  });
  const menu = useLayerController({ open: menuOpen, onOpenChange: setMenuOpen, canClose });
  const popover = useLayerController({ open: popoverOpen, onOpenChange: setPopoverOpen, canClose });
  const dialogHandoff = useMenuLayerHandoff({
    menu,
    layer: dialog,
    trigger: () => menuTrigger.current,
  });
  const sheetHandoff = useMenuLayerHandoff({
    menu,
    layer: sheet,
    trigger: () => menuTrigger.current,
  });
  const persistentFallback = () => fallback.current;
  const states = (
    <>
      <Badge>{busy ? 'Busy' : uncertain ? 'Uncertain' : 'Ready'}</Badge>
      <label>
        <input type="checkbox" checked={busy} onChange={(e) => setBusy(e.target.checked)} />
        Busy close veto
      </label>
      <label>
        <input
          type="checkbox"
          checked={uncertain}
          onChange={(e) => setUncertain(e.target.checked)}
        />
        Uncertain close veto
      </label>
    </>
  );
  return (
    <section aria-label="Layer qualification fixture" className="tmt:grid tmt:gap-4 tmt:p-4">
      <h2>Layers: private host state, no product operations</h2>
      <p>
        Controlled close admission is mandatory for these actionable stories. Raw state/unmount
        bypass it. Tooltip retains stock Radix behavior.
      </p>
      <Button ref={fallback}>Persistent layer fallback</Button>
      {states}
      <Button onClick={() => setOutsideActions((n) => n + 1)}>Underlying host action</Button>
      <Button
        onClick={() => {
          setMounted(true);
          setOpenerPresent(true);
        }}
      >
        Restore layer fixture
      </Button>
      <output data-layer-state="" aria-live="polite">
        {JSON.stringify({
          dialogOpen,
          sheetOpen,
          menuOpen,
          popoverOpen,
          outsideActions,
          menuActions,
          denied,
          dialogEntries,
          sheetEntries,
          menuCloseFocusEvents,
          lastClose,
          mounted,
        })}
      </output>
      {mounted && (
        <>
          <DropdownMenu controller={menu}>
            {openerPresent && (
              <DropdownMenuTrigger asChild>
                <Button ref={menuTrigger}>Persistent menu trigger</Button>
              </DropdownMenuTrigger>
            )}
            <DropdownMenuContent
              focusFallback={persistentFallback}
              onCloseAutoFocus={(event) => {
                dialogHandoff.onCloseAutoFocus(event);
                sheetHandoff.onCloseAutoFocus(event);
                setMenuCloseFocusEvents((count) => count + 1);
              }}
              aria-busy={busy || uncertain}
            >
              <DropdownMenuItem onSelect={dialogHandoff.onSelect}>
                Open Dialog from menu
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={sheetHandoff.onSelect}>
                Open Sheet from menu
              </DropdownMenuItem>
              <DropdownMenuItem
                onSelect={(event) => {
                  event.preventDefault();
                  if (menu.requestClose('selection', event)) setMenuActions((n) => n + 1);
                }}
              >
                Admitted fixture action
              </DropdownMenuItem>
              <DropdownMenuItem disabled>Unavailable action</DropdownMenuItem>
              <DropdownMenuCheckboxItem
                checked={busy}
                onCheckedChange={setBusy}
                onSelect={(event) => event.preventDefault()}
              >
                Menu busy close veto
              </DropdownMenuCheckboxItem>
              <DropdownMenuCheckboxItem
                checked={uncertain}
                onCheckedChange={setUncertain}
                onSelect={(event) => event.preventDefault()}
              >
                Menu uncertain close veto
              </DropdownMenuCheckboxItem>
              <DropdownMenuCheckboxItem checked={checked} onCheckedChange={setChecked}>
                Checked fixture value
              </DropdownMenuCheckboxItem>
              <DropdownMenuRadioGroup value={choice} onValueChange={setChoice}>
                <DropdownMenuRadioItem value="one">One</DropdownMenuRadioItem>
                <DropdownMenuRadioItem value="two">Two</DropdownMenuRadioItem>
              </DropdownMenuRadioGroup>
              <DropdownMenuSub>
                <DropdownMenuSubTrigger>Nested choices</DropdownMenuSubTrigger>
                <DropdownMenuSubContent>
                  <DropdownMenuItem>Nested item</DropdownMenuItem>
                </DropdownMenuSubContent>
              </DropdownMenuSub>
              <p>
                {busy
                  ? 'Busy: dismissal held'
                  : uncertain
                    ? 'Uncertain: dismissal held'
                    : 'Ready for selection'}
              </p>
            </DropdownMenuContent>
          </DropdownMenu>
          <Dialog controller={dialog}>
            {openerPresent && (
              <DialogTrigger asChild>
                <Button>Open fixture Dialog</Button>
              </DialogTrigger>
            )}
            <DialogContent
              focusFallback={persistentFallback}
              returnFocus={dialogHandoff.returnFocus}
              aria-busy={busy || uncertain}
            >
              <DialogHeader>
                <DialogTitle>Fixture Dialog title</DialogTitle>
                <DialogDescription>
                  Private controlled dialog and nested nonmodal popover.
                </DialogDescription>
              </DialogHeader>
              {states}
              <label>
                Private draft
                <input
                  value={draft}
                  readOnly={uncertain}
                  aria-readonly={uncertain}
                  onChange={(e) => setDraft(e.target.value)}
                />
              </label>
              <Popover controller={popover}>
                <PopoverTrigger asChild>
                  <Button>Nested fixture Popover</Button>
                </PopoverTrigger>
                <PopoverContent focusFallback={persistentFallback}>
                  <PopoverHeader>
                    <PopoverTitle>Nested Popover title</PopoverTitle>
                    <PopoverDescription>Nonmodal presentation only.</PopoverDescription>
                  </PopoverHeader>
                  <Button onClick={() => popover.requestClose()}>Programmatic Popover close</Button>
                </PopoverContent>
              </Popover>
              <Button onClick={() => setOpenerPresent(false)}>Remove persistent opener</Button>
              <Button onClick={() => setMounted(false)}>Unmount fixture layers</Button>
              <DialogFooter showCloseButton>
                <Button onClick={() => dialog.requestClose()}>Programmatic Dialog close</Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>
          <Sheet controller={sheet}>
            {openerPresent && (
              <SheetTrigger asChild>
                <Button>Open fixture Sheet</Button>
              </SheetTrigger>
            )}
            <SheetContent
              returnFocus={sheetHandoff.returnFocus}
              focusFallback={persistentFallback}
              aria-busy={busy || uncertain}
            >
              <SheetHeader>
                <SheetTitle>Fixture Sheet title</SheetTitle>
                <SheetDescription>Private scroll and viewport qualification.</SheetDescription>
              </SheetHeader>
              {states}
              <Button onClick={() => sheet.requestClose()}>Programmatic Sheet close</Button>
              <Button onClick={() => setOpenerPresent(false)}>Remove Sheet opener</Button>
              {Array.from({ length: 24 }, (_, i) => (
                <p key={i}>Private scroll row {i + 1}</p>
              ))}
            </SheetContent>
          </Sheet>
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button>Stock fixture Tooltip</Button>
              </TooltipTrigger>
              <TooltipContent>Display-only help; stock Radix Escape.</TooltipContent>
            </Tooltip>
          </TooltipProvider>
        </>
      )}
    </section>
  );
}

/** Allocated Slice3 fragment, exercised through the existing private foundation gallery. */
export function LayerFixtures() {
  return (
    <StrictMode>
      <LayerGallery />
    </StrictMode>
  );
}
