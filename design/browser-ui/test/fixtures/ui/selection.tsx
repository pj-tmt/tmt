import { useRef, useState } from 'react';
import {
  Combobox,
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
      className="tmt:grid tmt:gap-3 tmt:p-3 tmt:max-w-full"
    >
      <h2>Slice4 selection qualification</h2>
      <div className="tmt:flex tmt:flex-wrap tmt:gap-2">
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
        }}
        onClickCapture={(event) => {
          original.current = event.nativeEvent;
        }}
        onKeyDown={(event) => {
          if (event.key === 'Enter') setSend((count) => count + 1);
        }}
      >
        <Button>Before selection</Button>
        <Combobox
          controller={controller}
          label="Selection search"
          query={query}
          onQueryChange={setQuery}
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
      <ChipsCase />
      <output aria-label="Selection value">{value ?? 'None'}</output>
      <output aria-label="Selection callback count">{calls}</output>
      <output aria-label="Selection original event">{eventProof}</output>
      <output aria-label="Selection send count">{send}</output>
      <output aria-label="Selection outside count">{outside}</output>
      <output aria-label="Selection open">{String(open)}</output>
    </section>
  );
}

function ChipsCase() {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [items, setItems] = useState([
    { id: 'alpha', label: 'Alpha', removeLabel: 'Remove Alpha' },
  ]);
  const [outside, setOutside] = useState(0);
  const [adds, setAdds] = useState(0);
  const [removes, setRemoves] = useState(0);
  const [send, setSend] = useState(0);
  const [blocked, setBlocked] = useState(false);
  const [proof, setProof] = useState('None');
  const controller = useLayerController({ open, onOpenChange: setOpen, canClose: () => !blocked });
  return (
    <section
      aria-label="Chips qualification"
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') setSend((count) => count + 1);
      }}
    >
      <Button>Before chips</Button>
      <Button onClick={() => setBlocked(!blocked)}>Chips busy {blocked ? 'on' : 'off'}</Button>
      <Button onClick={() => controller.requestOpen()}>Open chips programmatically</Button>
      <Chips
        controller={controller}
        label="Chips search"
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
        busy={blocked}
        stateLabel={
          blocked
            ? 'Working: draft retained'
            : items.length >= 2
              ? 'Capacity reached: remove a chip to add another'
              : undefined
        }
      />
      <Button>After chips</Button>
      <Button onClick={() => setOutside((count) => count + 1)}>Chips outside action</Button>
      <output aria-label="Chips outside count">{outside}</output>
      <output aria-label="Chips add count">{adds}</output>
      <output aria-label="Chips remove count">{removes}</output>
      <output aria-label="Chips Send count">{send}</output>
      <output aria-label="Chips event">{proof}</output>
    </section>
  );
}
