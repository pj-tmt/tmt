import type { ReactNode } from "react";
import { useStrings } from "../lang/useStrings";

// The four scenes of the start journey. Each is one complete picture; the
// journey adds a layer to the same foundation at every step.

const mini =
  "min-w-0 overflow-hidden rounded-md border border-term-edge bg-term p-3 font-mono text-[12.5px] leading-[1.55] text-t-text";

function Pane({
  name,
  driver,
  tone,
  children,
}: {
  name: string;
  driver: string;
  tone: string;
  children: ReactNode;
}) {
  return (
    <div className={mini}>
      <div className="mb-1.5 flex justify-between text-t-muted">
        <b className="text-t-text">{name}</b>
        <span className={tone}>{driver}</span>
      </div>
      {children}
    </div>
  );
}

const Arrow = ({ children }: { children: ReactNode }) => (
  <div className="col-span-full text-center font-mono text-xs font-semibold text-waiting">
    {children}
  </div>
);

const New = ({ children }: { children: ReactNode }) => (
  <span className="ml-1.5 rounded-[3px] bg-t-accent px-1.5 py-0.5 text-[10px] font-semibold text-term-bar">
    {children}
  </span>
);

export function TalkScene() {
  const { scenes } = useStrings().journey;
  return (
    <div className="grid grid-cols-1 content-center gap-3 sm:grid-cols-2">
      <Pane name="lead" driver="claude" tone="text-t-review">
        <span className="text-t-accent">$ </span>tmt talk builder "add a login test"
        <br />
        <span className="text-t-dim">waiting for builder…</span>
        <br />
        <br />
        <span className="text-t-working">✓ </span>Test added, 12 passed
      </Pane>
      <Pane name="builder" driver="codex" tone="text-t-link">
        <span className="text-t-dim">{'<tmt-reply from="lead">'}</span>
        <br />
        add a login test
        <br />
        <span className="text-t-dim">…working…</span>
        <br />
        <span className="text-t-accent">$ </span>tmt reply req_9ba4… --receipt v2_…
      </Pane>
      <Arrow>{scenes.talkArrow}</Arrow>
    </div>
  );
}

export function BoardStepScene() {
  const { scenes } = useStrings().journey;
  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
      <div className={mini}>
        <div className="mb-1.5 flex justify-between text-t-muted">
          <b className="text-t-text">tmt sq board</b>
          <New>{scenes.boardNew}</New>
        </div>
        <pre className="m-0 font-mono text-[12.5px] leading-[1.55] whitespace-pre-wrap">
          <span className="text-t-working">●</span>
          {" builder   "}
          <span className="text-t-working">working</span>
          {"  login test\n"}
          <span className="text-t-waiting">◆</span>
          {" reviewer  review   #412\n"}
          <span className="text-t-working">●</span>
          {" tester    "}
          <span className="text-t-working">working</span>
          {"  e2e\n"}
          <span className="text-t-dim">○ docs idle –</span>
        </pre>
      </div>
      <div className="grid content-start gap-3">
        <Pane name="lead" driver="claude" tone="text-t-review">
          <span className="text-t-waiting">[tmt] reply from builder</span>
        </Pane>
        <Pane name="builder" driver="codex" tone="text-t-link">
          <span className="text-t-working">✓ </span>test added
        </Pane>
        <Arrow>{scenes.boardArrow}</Arrow>
      </div>
    </div>
  );
}

function Machine({ name, children }: { name: string; children: ReactNode }) {
  return (
    <div className="min-w-0 overflow-hidden rounded-lg border border-term-edge bg-term font-mono text-xs leading-[1.55] text-t-text">
      <div className="flex justify-between bg-term-bar px-2.5 py-1.5 font-semibold text-t-muted">
        <span>{name}</span>
        <span className="text-t-working">●</span>
      </div>
      <div className="p-2.5">{children}</div>
    </div>
  );
}

export function ColabScene() {
  const { scenes } = useStrings().journey;
  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
      <Machine name={scenes.yourLaptop}>
        <span className="text-t-working">●</span> lead
        <br />
        <span className="text-t-working">●</span> builder
        <br />
        <span className="text-t-waiting">◆</span> reviewer
      </Machine>
      <Machine name={scenes.buildServer}>
        <span className="text-t-working">●</span> tester
        <br />
        <span className="text-t-working">●</span> perf
      </Machine>
      <Machine name={scenes.teammate}>
        <span className="text-t-link">◇</span> Mei{" "}
        <span className="text-t-muted">{scenes.human}</span>
        <br />
        <span className="text-t-working">●</span> {scenes.herLead}
      </Machine>
      <div className="col-span-full text-center font-mono text-[11px] font-semibold text-accent">
        ⇣ {scenes.sharedPage} <New>{scenes.inProgress}</New>
      </div>
      <div className="col-span-full overflow-hidden rounded-lg border border-rule bg-sheet text-text">
        <div className="flex items-center gap-1.5 bg-rule px-2.5 py-1.5 font-mono text-[11px] text-muted">
          <i className="size-2 rounded-full bg-dim" />
          <i className="size-2 rounded-full bg-dim" />
          <i className="size-2 rounded-full bg-dim" />
          <span className="ml-1.5">{scenes.pageTitle}</span>
        </div>
        <div className="grid grid-cols-1 gap-3 p-3 text-[13px] leading-normal sm:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
          <div>
            <h5 className="m-0 mb-1 font-body text-[15px] font-semibold">{scenes.planHeading}</h5>
            1. {scenes.planOne} <span className="text-working">✓</span>
            <br />
            2. {scenes.planTwo} <span className="text-working">✓</span>
            <br />
            3. {scenes.planThree} <span className="text-waiting">◆ {scenes.decide}</span>
          </div>
          <div className="grid gap-1.5 text-[12.5px]">
            <div className="border-l-[3px] border-rule pl-2">
              <b className="font-mono text-[11px]">reviewer</b>
              <br />
              {scenes.reviewerSays}
            </div>
            <div className="border-l-[3px] border-rule pl-2">
              <b className="font-mono text-[11px]">Mei</b>
              <br />
              {scenes.meiSays}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function Seat({
  initial,
  name,
  tone,
  talking,
  hand,
  speaking,
}: {
  initial: string;
  name: string;
  tone: string;
  talking?: boolean;
  hand?: string;
  speaking?: string;
}) {
  return (
    <div
      className={`relative rounded-lg border bg-term p-3 text-center font-mono text-xs text-t-text ${
        talking ? "border-t-accent ring-4 ring-t-accent/25" : "border-term-edge"
      }`}
    >
      {hand && <span className="absolute top-1.5 right-2 font-bold text-t-waiting">{hand}</span>}
      <div
        className={`mx-auto mb-1.5 grid size-9 place-items-center rounded-full text-[15px] font-bold text-term-bar ${tone}`}
      >
        {initial}
      </div>
      {name}
      {talking && (
        <>
          <br />
          <span className="text-t-working">{speaking}</span>
        </>
      )}
    </div>
  );
}

export function MeetScene() {
  const { scenes } = useStrings().journey;
  return (
    <div className="grid content-center gap-3">
      <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-5">
        <Seat initial="Y" name={scenes.youHost} tone="bg-t-text" />
        <Seat initial="L" name="lead" tone="bg-t-review" />
        <Seat initial="B" name="builder" tone="bg-t-link" talking speaking={scenes.speaking} />
        <Seat initial="R" name="reviewer" tone="bg-t-review" hand="↑1" />
        <Seat initial="M" name="Mei" tone="bg-t-working" hand="↑2" />
      </div>
      <div className="rounded-lg border border-term-edge bg-term px-3 py-2.5 font-mono text-[13px] leading-[1.55] text-t-text">
        <b className="text-t-accent">builder</b> · {scenes.meetSays}
      </div>
      <div className="text-center font-mono text-xs font-semibold text-waiting">
        {scenes.meetArrow} <New>{scenes.planned}</New>
      </div>
    </div>
  );
}
