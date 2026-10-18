import { useRef, useState, type SyntheticEvent } from 'react';
import { Slot } from 'radix-ui';
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
} from '../../../src/ui/primitives.js';

/** Real slotted capture handlers: local counters only, no product activation. */
function SlottedCaptureFixture() {
  const [blockedEffects, setBlockedEffects] = useState(0);
  const [childEffects, setChildEffects] = useState(0);
  const [hostEffects, setHostEffects] = useState(0);
  const [forwarded, setForwarded] = useState(false);
  const childRef = useRef<HTMLAnchorElement>(null);
  const hostRef = useRef<HTMLElement | null>(null);
  const originalEvent = useRef<Event | null>(null);
  const childCapture = (event: SyntheticEvent) => {
    originalEvent.current = event.nativeEvent;
    setChildEffects((count) => count + 1);
  };
  const hostCapture = (event: SyntheticEvent) => {
    setForwarded(
      event.isTrusted &&
        event.nativeEvent === originalEvent.current &&
        event.currentTarget === childRef.current &&
        event.currentTarget === hostRef.current,
    );
    setHostEffects((count) => count + 1);
  };
  return (
    <section aria-label="Slotted capture qualification" className="tmt:grid tmt:gap-2">
      <Button
        asChild
        disabled
        ref={(node) => node?.setAttribute('data-host-ref', 'yes')}
        onClick={() => setBlockedEffects((count) => count + 1)}
        onClickCapture={() => setBlockedEffects((count) => count + 1)}
        onKeyDownCapture={() => setBlockedEffects((count) => count + 1)}
      >
        <a
          ref={(node) => node?.setAttribute('data-child-ref', 'yes')}
          href="#blocked-capture-navigation"
          onClick={() => setBlockedEffects((count) => count + 1)}
          onClickCapture={() => setBlockedEffects((count) => count + 1)}
          onKeyDownCapture={(event) => {
            if (event.key === 'Enter' || event.key === ' ') setBlockedEffects((count) => count + 1);
          }}
        >
          Disabled capture child
        </a>
      </Button>
      {(['children', 'render'] as const).map((form) => {
        const target = (
          <a
            ref={(node) => node?.setAttribute('data-child-ref', 'yes')}
            href="#blocked-slottable-navigation"
            onClickCapture={() => setBlockedEffects((count) => count + 1)}
            onKeyDownCapture={(event) => {
              if (event.key === 'Enter' || event.key === ' ')
                setBlockedEffects((count) => count + 1);
            }}
          >
            Disabled Slottable {form}
          </a>
        );
        return (
          <Button
            asChild
            disabled
            key={form}
            ref={(node) => node?.setAttribute('data-host-ref', 'yes')}
          >
            {form === 'children' ? (
              <Slot.Slottable>{target}</Slot.Slottable>
            ) : (
              <Slot.Slottable child={target}>{(content) => content}</Slot.Slottable>
            )}
          </Button>
        );
      })}
      <Button
        asChild
        ref={(node) => {
          hostRef.current = node;
        }}
        onClickCapture={hostCapture}
        onKeyDownCapture={(event) => {
          if (event.key === 'Enter' || event.key === ' ') hostCapture(event);
        }}
      >
        <a
          ref={childRef}
          href="#enabled-capture-navigation"
          onClick={(event) => event.preventDefault()}
          onClickCapture={childCapture}
          onKeyDownCapture={(event) => {
            if (event.key === 'Enter' || event.key === ' ') childCapture(event);
          }}
        >
          Enabled capture child
        </a>
      </Button>
      <output aria-label="Blocked capture effects">{blockedEffects}</output>
      <output aria-label="Enabled child capture effects">{childEffects}</output>
      <output aria-label="Enabled host capture effects">{hostEffects}</output>
      <output aria-label="Original event and refs forwarded">{String(forwarded)}</output>
    </section>
  );
}

/** Isolated presentation state only; no product store, service or sends. */
export function PrimitiveFixtures() {
  const [actions, setActions] = useState(0);
  const [text, setText] = useState('Kept input');
  const [message, setMessage] = useState('Kept textarea');
  const [checked, setChecked] = useState<boolean | 'indeterminate'>(false);
  const [enabled, setEnabled] = useState(false);
  const [tab, setTab] = useState('first');
  return (
    <section
      aria-label="Real shadcn primitives"
      className="tmt:font-body tmt:text-(length:--tmt-foundation-command-size) tmt:grid tmt:gap-4 tmt:p-4 tmt:bg-background tmt:text-foreground"
    >
      <h3>Slice2 primitive qualification</h3>
      <SlottedCaptureFixture />
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
      {/* #3388: uncontrolled controls associated with a form they are not inside. */}
      <form id="primitive-external-form" aria-label="External reset form">
        <label>
          <Checkbox aria-label="In-form checkbox" name="in-form-check" /> In-form checkbox
        </label>
        <Button type="reset">Reset external form</Button>
      </form>
      <div className="tmt:flex tmt:flex-wrap tmt:gap-4">
        <label>
          <Checkbox
            aria-label="External-form checkbox"
            name="external-check"
            form="primitive-external-form"
          />{' '}
          External-form checkbox
        </label>
        <label>
          <Switch
            aria-label="External-form switch"
            name="external-switch"
            form="primitive-external-form"
          />{' '}
          External-form switch
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
      <Button type="button" variant="outline" onClick={() => setActions((count) => count + 1)}>
        Additional action
      </Button>
    </section>
  );
}
