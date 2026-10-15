import { StrictMode, useRef, useState } from 'react';
import {
  Combobox,
  Command,
  CommandInput,
  CommandList,
  CommandItem,
  PopoverAnchor,
  Dialog,
  DialogContent,
  DialogTitle,
  DialogTrigger,
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  Popover,
  PopoverTrigger,
  PopoverContent,
  PopoverTitle,
  Chips,
  Button,
  useLayerController,
  type SelectionEvent,
} from '../../../src/ui/index.js';

const choices = [
  { id: 'alpha', label: 'Alpha' },
  { id: 'beta', label: 'Beta' },
  { id: 'blocked', label: 'Unavailable option', disabled: true },
  { id: 'gamma', label: 'Gamma with a deliberately long label that wraps in narrow viewports' },
];
export function SelectionFixtures() {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [value, setValue] = useState<string | null>(null);
  const [veto, setVeto] = useState(false);
  const [busy, setBusy] = useState(false);
  const [readOnly, setReadOnly] = useState(false);
  const [calls, setCalls] = useState(0);
  const [send, setSend] = useState(0);
  const [outside, setOutside] = useState(0);
  const [eventProof, setEventProof] = useState('None');
  const [removed, setRemoved] = useState(false);
  const [prevent, setPrevent] = useState(false);
  const [keyDetail, setKeyDetail] = useState('None');
  const original = useRef<Event | null>(null);
  const controller = useLayerController({ open, onOpenChange: setOpen, canClose: () => !veto });
  function selected(event: SelectionEvent) {
    setCalls((count) => count + 1);
    setEventProof(
      `${event.nativeEvent.type}:${event.nativeEvent.isTrusted}:${event.nativeEvent === original.current}`,
    );
  }
  return (
    <section
      aria-label="Selection qualification"
      className="tmt:grid tmt:gap-3 tmt:p-3 tmt:max-w-full tmt:bg-background tmt:text-foreground tmt:font-body tmt:text-(length:--tmt-foundation-command-size)"
    >
      <h3>Slice4 selection qualification</h3>
      <div className="tmt:flex tmt:flex-wrap tmt:gap-2">
        <Button onClick={() => setPrevent(!prevent)}>
          Prevent selection keys {prevent ? 'on' : 'off'}
        </Button>
        <Button onClick={() => setVeto(!veto)}>Close veto {veto ? 'on' : 'off'}</Button>
        <Button onClick={() => setBusy(!busy)}>Selection busy {busy ? 'on' : 'off'}</Button>
        <Button onClick={() => setReadOnly(!readOnly)}>
          Selection read-only {readOnly ? 'on' : 'off'}
        </Button>
        <Button onClick={() => setRemoved(!removed)}>
          Remove active option {removed ? 'on' : 'off'}
        </Button>
        <Button onClick={() => controller.requestOpen()}>Open selection programmatically</Button>
        <Button onClick={() => controller.requestClose()}>Close selection programmatically</Button>
      </div>
      <div
        onKeyDownCapture={(event) => {
          original.current = event.nativeEvent;
          setKeyDetail(
            `${event.nativeEvent.isTrusted}:${event.nativeEvent.isComposing}:${event.keyCode}`,
          );
        }}
        onClickCapture={(event) => {
          original.current = event.nativeEvent;
        }}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') setSend((count) => count + 1);
        }}
      >
        <Button>Before selection</Button>
        <Combobox
          controller={controller}
          label="Selection search"
          query={query}
          onQueryChange={setQuery}
          onKeyDown={(event) => {
            if (prevent && event.key === 'Enter') event.preventDefault();
          }}
          options={choices.filter(
            (option) =>
              (!removed || option.id !== 'alpha') &&
              option.label.toLowerCase().includes(query.toLowerCase()),
          )}
          value={value}
          onSelect={(option, event) => {
            selected(event);
            setValue(option.id);
          }}
          emptyLabel="No matching choices"
          busy={busy}
          readOnly={readOnly}
          stateLabel={
            busy
              ? 'Working: selection held'
              : readOnly
                ? 'Read-only: selection retained'
                : veto
                  ? 'Close veto: draft retained'
                  : undefined
          }
        />
        <Button>After selection</Button>
        <Button onClick={() => setOutside((count) => count + 1)}>Selection outside action</Button>
      </div>
      <StrictMode>
        <ChipsCase />
      </StrictMode>
      <NestedCase />
      <DescendantLayersCase />
      <output aria-label="Selection key detail">{keyDetail}</output>
      <output aria-label="Selection value">{value ?? 'None'}</output>
      <output aria-label="Selection callback count">{calls}</output>
      <output aria-label="Selection original event">{eventProof}</output>
      <output aria-label="Selection send count">{send}</output>
      <output aria-label="Selection outside count">{outside}</output>
      <output aria-label="Selection open">{String(open)}</output>
    </section>
  );
}

function ChipsCase({ nested = false }: { nested?: boolean }) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [items, setItems] = useState([
    { id: 'alpha', label: 'Alpha', removeLabel: 'Remove Alpha' },
  ]);
  const [outside, setOutside] = useState(0);
  const [adds, setAdds] = useState(0);
  const [removes, setRemoves] = useState(0);
  const [send, setSend] = useState(0);
  const [clicks, setClicks] = useState(0);
  const [blocked, setBlocked] = useState(false);
  const [proof, setProof] = useState('None');
  const [mounted, setMounted] = useState(true);
  const [readOnly, setReadOnly] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const fallback = useRef<HTMLButtonElement>(null);

  const controller = useLayerController({
    open,
    onOpenChange: setOpen,
    canClose: () => !blocked && !uncertain,
  });
  return (
    <section
      aria-label="Chips qualification"
      onClick={() => setClicks((count) => count + 1)}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') setSend((count) => count + 1);
      }}
    >
      <Button>Before chips</Button>
      <Button onClick={() => setBlocked(!blocked)}>Chips busy {blocked ? 'on' : 'off'}</Button>
      <Button onClick={() => controller.requestOpen()}>Open chips programmatically</Button>
      <Button onClick={() => setMounted(!mounted)}>Chips mounted {mounted ? 'on' : 'off'}</Button>
      <Button onClick={() => setReadOnly(!readOnly)}>
        Chips read-only {readOnly ? 'on' : 'off'}
      </Button>
      <Button onClick={() => setUncertain(!uncertain)}>
        Chips unknown {uncertain ? 'on' : 'off'}
      </Button>
      {mounted && (
        <Chips
          controller={controller}
          label={nested ? 'Nested chips search' : 'Chips search'}
          focusFallback={() => fallback.current}
          query={query}
          onQueryChange={setQuery}
          items={items}
          options={
            items.length >= 2
              ? []
              : choices.filter(
                  (option) =>
                    !items.some((item) => item.id === option.id) &&
                    option.label.toLowerCase().includes(query.toLowerCase()),
                )
          }
          onAdd={(option, event) => {
            setAdds((count) => count + 1);
            setProof(`${event.nativeEvent.type}:${event.nativeEvent.isTrusted}`);
            setItems((values) => [
              ...values,
              { id: option.id, label: option.label, removeLabel: `Remove ${option.label}` },
            ]);
            setQuery('');
          }}
          onRemove={(item, event) => {
            setRemoves((count) => count + 1);
            setProof(`${event.nativeEvent.type}:${event.nativeEvent.isTrusted}`);
            setItems((values) => values.filter((value) => value.id !== item.id));
          }}
          emptyLabel="No eligible choices"
          readOnly={readOnly}
          busy={blocked || uncertain}
          stateLabel={
            uncertain
              ? 'Unknown: outcome uncertain; draft retained'
              : readOnly
                ? 'Read-only: retained chips'
                : blocked
                  ? 'Working: draft retained'
                  : items.length >= 2
                    ? 'Capacity reached: remove a chip to add another'
                    : undefined
          }
        />
      )}
      <Button ref={fallback}>After chips</Button>
      <Button
        className="tmt:fixed tmt:right-0 tmt:top-0"
        onClick={() => setOutside((count) => count + 1)}
      >
        Chips outside action
      </Button>
      <output aria-label="Chips outside count">{outside}</output>
      <output aria-label="Chips add count">{adds}</output>
      <output aria-label="Chips remove count">{removes}</output>
      <output aria-label="Chips parent clicks">{clicks}</output>
      <output aria-label="Chips Send count">{send}</output>
      <output aria-label="Chips event">{proof}</output>
    </section>
  );
}

function NestedCase() {
  const [open, setOpen] = useState(false);
  const controller = useLayerController({ open, onOpenChange: setOpen });
  return (
    <Dialog controller={controller}>
      <DialogTrigger asChild>
        <Button>Selection dialog</Button>
      </DialogTrigger>
      <DialogContent>
        <DialogTitle>Nested selection dialog</DialogTitle>
        <ChipsCase nested />
      </DialogContent>
    </Dialog>
  );
}

// Genuine React descendants, not sibling roots: the public layer composition
// exercises its shield/nesting contract. Actual Chips-in-Dialog is separate.
function DescendantLayersCase() {
  const [open, setOpen] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [childOpen, setChildOpen] = useState(false);
  const [query, setQuery] = useState('Retained nested query');
  const input = useRef<HTMLInputElement>(null);
  const controller = useLayerController({ open, onOpenChange: setOpen });
  const menu = useLayerController({ open: menuOpen, onOpenChange: setMenuOpen });
  const child = useLayerController({ open: childOpen, onOpenChange: setChildOpen });
  return (
    <Command label="Descendant input" shouldFilter={false}>
      <Popover controller={controller}>
        <PopoverAnchor asChild>
          <div className="tmt:relative tmt:z-[calc(var(--tmt-foundation-z-layer)+1)]">
            <CommandInput asChild value={query} onValueChange={setQuery}>
              <input ref={input} aria-label="Descendant input" aria-expanded={open} />
            </CommandInput>
          </div>
        </PopoverAnchor>
        <PopoverTrigger asChild>
          <Button>Open descendant selection layer</Button>
        </PopoverTrigger>
        <PopoverContent
          outsidePress="dismiss-only"
          initialFocus={() => input.current}
          returnFocus={() => input.current}
          onInteractOutside={(event) => {
            if (event.detail.originalEvent.target === input.current) event.preventDefault();
          }}
        >
          <PopoverTitle>Descendant parent layer</PopoverTitle>
          <CommandList>
            <CommandItem value="nested">Retained nested option</CommandItem>
          </CommandList>
          <DropdownMenu controller={menu}>
            <DropdownMenuTrigger asChild>
              <Button>Descendant menu trigger</Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent>
              <DropdownMenuItem>Descendant menu action</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <Popover controller={child}>
            <PopoverTrigger asChild>
              <Button>Descendant Popover trigger</Button>
            </PopoverTrigger>
            <PopoverContent>
              <PopoverTitle>Descendant child Popover</PopoverTitle>
              <Button>Descendant Popover action</Button>
            </PopoverContent>
          </Popover>
        </PopoverContent>
      </Popover>
    </Command>
  );
}
