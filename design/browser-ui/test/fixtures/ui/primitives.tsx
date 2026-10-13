import { useRef, useState } from 'react';
import {
  Button,
  Input,
  Textarea,
  Checkbox,
  Switch,
  Separator,
  Badge,
  Alert,
  AlertTitle,
  AlertDescription,
  Tabs,
  TabsList,
  TabsTrigger,
  TabsContent,
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectGroup,
  SelectLabel,
  SelectItem,
  SelectSeparator,
} from '../../../src/ui/primitives.js';

/** Isolated presentation state only; no product store, service or sends. */
export function PrimitiveFixtures() {
  const [actions, setActions] = useState(0);
  const [text, setText] = useState('Kept input');
  const [message, setMessage] = useState('Kept textarea');
  const [checked, setChecked] = useState<boolean | 'indeterminate'>(false);
  const [enabled, setEnabled] = useState(false);
  const [tab, setTab] = useState('first');
  const [choice, setChoice] = useState('alpha');
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [readOnly, setReadOnly] = useState(false);
  const [triggerMounted, setTriggerMounted] = useState(true);
  const [changes, setChanges] = useState(0);
  const trigger = useRef<HTMLButtonElement>(null);
  const fallback = useRef<HTMLButtonElement>(null);
  const requestOpen = (next: boolean) => {
    if (!busy) setOpen(next);
  };
  return (
    <section
      aria-label="Real shadcn primitives"
      className="tmt:grid tmt:gap-4 tmt:p-4 tmt:bg-background tmt:text-foreground"
    >
      <h3>Slice2 primitive qualification</h3>
      <div className="tmt:flex tmt:flex-wrap tmt:gap-2">
        <Button type="button" onClick={() => setActions((count) => count + 1)}>
          Ready action
        </Button>
        <Button type="button" disabled>
          Disabled action
        </Button>
        <Button
          type="button"
          busy
          busyLabel="Working action"
          onClick={() => setActions((count) => count + 1)}
        />
        <Button asChild disabled>
          <a href="#should-not-navigate" onClick={() => setActions((count) => count + 1)}>
            Disabled child action
          </a>
        </Button>
        <output aria-label="Action count">{actions}</output>
        {(['default', 'destructive', 'outline', 'secondary', 'ghost', 'link'] as const).map(
          (variant) => (
            <Button type="button" key={variant} variant={variant}>
              Button {variant}
            </Button>
          ),
        )}
        {(['default', 'xs', 'sm', 'lg', 'icon', 'icon-xs', 'icon-sm', 'icon-lg'] as const).map(
          (size) => (
            <Button type="button" key={size} size={size} aria-label={`Size ${size}`}>
              {size.startsWith('icon') ? '+' : `Size ${size}`}
            </Button>
          ),
        )}
      </div>
      <label>
        Controlled input
        <Input value={text} onChange={(event) => setText(event.target.value)} />
      </label>
      <label>
        Controlled textarea
        <Textarea value={message} onChange={(event) => setMessage(event.target.value)} />
      </label>
      <label>
        Read-only input
        <Input readOnly value="Read-only value" />
      </label>
      <label>
        Read-only textarea
        <Textarea readOnly value="Read-only text" />
      </label>
      <label>
        Disabled input
        <Input disabled value="Disabled value" />
      </label>
      <label>
        Disabled textarea
        <Textarea disabled value="Disabled text" />
      </label>
      <label>
        Invalid input
        <Input
          aria-invalid="true"
          aria-describedby="primitive-error"
          defaultValue="Invalid value"
        />
      </label>
      <p id="primitive-error">Invalid: supply a valid value.</p>
      <label>
        Working input
        <Input
          aria-busy="true"
          aria-describedby="primitive-working"
          readOnly
          value="Retained pending value"
        />
      </label>
      <p id="primitive-working" role="status">
        Working; edits are paused.
      </p>
      <label>
        Working textarea
        <Textarea
          readOnly
          aria-busy="true"
          aria-describedby="primitive-working"
          value="Retained pending text"
        />
      </label>
      <div className="tmt:flex tmt:flex-wrap tmt:gap-4">
        <label>
          <Checkbox
            aria-label="Controlled checkbox"
            checked={checked}
            onCheckedChange={setChecked}
          />{' '}
          Controlled checkbox: {String(checked)}
        </label>
        <label>
          <Checkbox aria-label="Read-only checkbox" readOnly defaultChecked /> Read-only checkbox
        </label>
        <label>
          <Checkbox aria-label="Disabled checkbox" disabled /> Disabled checkbox
        </label>
        <label>
          <Checkbox aria-label="Mixed checkbox" checked="indeterminate" /> Mixed checkbox
        </label>
        <label>
          <Checkbox aria-label="Working checkbox" disabled aria-busy="true" /> Working checkbox
        </label>
        <label>
          <Switch aria-label="Controlled switch" checked={enabled} onCheckedChange={setEnabled} />{' '}
          Controlled switch: {enabled ? 'On' : 'Off'}
        </label>
        <label>
          <Switch aria-label="Read-only switch" readOnly defaultChecked /> Read-only switch
        </label>
        <label>
          <Switch aria-label="Disabled switch" disabled /> Disabled switch
        </label>
        <label>
          <Switch aria-label="Working switch" disabled aria-busy="true" /> Working switch
        </label>
      </div>
      <Separator decorative={false} />
      <div className="tmt:flex tmt:flex-wrap tmt:gap-2">
        {(['default', 'secondary', 'destructive', 'outline', 'ghost', 'link'] as const).map(
          (variant) => (
            <Badge key={variant} variant={variant}>
              Badge {variant}
            </Badge>
          ),
        )}
      </div>
      <Alert>
        <AlertTitle>Ready information</AlertTitle>
        <AlertDescription>
          Literal host information, without product state inference.
        </AlertDescription>
      </Alert>
      <Alert variant="destructive">
        <AlertTitle>Failed action</AlertTitle>
        <AlertDescription>Failure retains its visible words.</AlertDescription>
      </Alert>
      <Tabs value={tab} onValueChange={setTab} activationMode="manual">
        <TabsList aria-label="Horizontal manual tabs">
          <TabsTrigger value="first">First</TabsTrigger>
          <TabsTrigger value="disabled" disabled>
            Disabled tab
          </TabsTrigger>
          <TabsTrigger value="second">Second</TabsTrigger>
        </TabsList>
        <TabsContent value="first">First panel</TabsContent>
        <TabsContent value="second">Second panel</TabsContent>
      </Tabs>
      <Tabs defaultValue="north" orientation="vertical">
        <TabsList aria-label="Vertical automatic tabs" variant="line">
          <TabsTrigger value="north">North</TabsTrigger>
          <TabsTrigger value="south">South</TabsTrigger>
        </TabsList>
        <TabsContent value="north">North panel</TabsContent>
        <TabsContent value="south">South panel</TabsContent>
      </Tabs>
      <div className="tmt:flex tmt:flex-wrap tmt:gap-2">
        <Button type="button" ref={fallback} onClick={() => requestOpen(true)}>
          Open select programmatically
        </Button>
        <Button type="button" variant="outline" onClick={() => requestOpen(false)}>
          Close select programmatically
        </Button>
        <Button type="button" variant="outline" onClick={() => setBusy(!busy)}>
          {busy ? 'Finish select work' : 'Start select work'}
        </Button>
        <Button type="button" variant="outline" onClick={() => setReadOnly(!readOnly)}>
          {readOnly ? 'Enable choice edits' : 'Make choice read-only'}
        </Button>
        <Button type="button" variant="outline" onClick={() => setTriggerMounted(!triggerMounted)}>
          {triggerMounted ? 'Remove select opener' : 'Restore select opener'}
        </Button>
      </div>
      <p role="status">
        {busy
          ? 'Select working; dismissal is paused.'
          : readOnly
            ? 'Select read-only; value is retained.'
            : 'Select ready.'}
      </p>
      <Select
        value={choice}
        open={open}
        readOnly={readOnly || busy}
        onOpenChange={requestOpen}
        onValueChange={(next) => {
          setChoice(next);
          setChanges((count) => count + 1);
        }}
      >
        <div data-primitive-anchor-clip="" className="tmt:overflow-hidden tmt:h-9">
          {triggerMounted && (
            <SelectTrigger
              ref={trigger}
              aria-label="Controlled choice"
              aria-busy={busy || undefined}
            >
              <SelectValue />
            </SelectTrigger>
          )}
        </div>
        <SelectContent
          aria-busy={busy || undefined}
          onEscapeKeyDown={(event) => {
            if (busy) event.preventDefault();
          }}
          onPointerDownOutside={(event) => {
            if (busy) event.preventDefault();
          }}
          onCloseAutoFocus={(event) => {
            if (!trigger.current?.isConnected) {
              event.preventDefault();
              fallback.current?.focus();
            }
          }}
        >
          <SelectGroup>
            <SelectLabel>
              {busy ? 'Working choices' : readOnly ? 'Read-only choices' : 'Real choices'}
            </SelectLabel>
            <SelectItem value="alpha">Alpha</SelectItem>
            <SelectItem value="blocked" disabled>
              Disabled choice
            </SelectItem>
            <SelectItem value="beta">Beta</SelectItem>
            <SelectSeparator />
            {Array.from({ length: 12 }, (_, i) => (
              <SelectItem key={i} value={`long-${i}`}>
                Long choice {i + 1}: retained full accessible name
              </SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
      <output aria-label="Choice and change count">
        {choice}:{changes}
      </output>
      <Select disabled defaultValue="fixed">
        <SelectTrigger aria-label="Disabled choice">
          <SelectValue>Disabled choice</SelectValue>
        </SelectTrigger>
      </Select>
      <Button type="button" variant="outline" onClick={() => setActions((count) => count + 1)}>
        Outside action
      </Button>
    </section>
  );
}
