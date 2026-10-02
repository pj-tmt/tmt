import { useRef } from "react";
import type { ReactNode } from "react";
import { FitWidth } from "./FitWidth";
import { DemoControls, useStepPlayer } from "./player";

// Three tmux panes running shells and agent CLIs, animated step by step.
type Item =
  | string
  | { c: string } // a shell command
  | { d: string } // dim
  | { w: string } // waiting
  | { x: string } // error
  | { o: string } // success
  | { u: string } // the user's message in an agent
  | { a: string } // the agent speaking
  | { t: string } // a tool call
  | { r: string } // a tool result
  | { k: string }; // the agent's work timer
type Agent = "claude" | "codex";
type Badge = { n: string; s: "on" | "ended" | "none" };
type Pane = { ui?: Agent; launch?: string; badge?: Badge; lines?: Item[] };
type Step = { focus: number; panes: Pane[]; cap: string; ms?: number };
type Layout = { id: string; tall?: boolean }[];

type PaneFrame = { ui?: Agent; badge?: Badge; lines: Item[]; typing?: string; cursor: boolean };
type Frame = { focus: number; panes: PaneFrame[] };

const LOOK = {
  claude: {
    head: (
      <div className="mx-2 mt-1.5 mb-1 flex-none rounded border border-[#E0AF68]/55 px-2 py-1 leading-[1.45]">
        <span className="text-[#E0AF68]">✻</span> Welcome to <b>Claude Code</b>
        <br />
        <span className="text-t-dim">cwd: ~/web</span>
      </div>
    ),
    prompt: <span className="text-t-dim">❯ </span>,
    user: "❯ ",
    say: "● ",
    tool: (text: string) => (
      <>
        ● <span className="font-semibold">Bash</span>(<span className="text-t-accent">{text}</span>)
      </>
    ),
    result: "  ⎿  ",
    work: "✻ ",
    input: "mt-1.5 border-y border-t-dim/70 px-2.5 py-[3px]",
    placeholder: "",
    foot: "Opus · ~/web · ctx 12%",
    shellFoot: <span className="text-t-blocked">! for shell mode</span>,
  },
  codex: {
    head: (
      <div className="mx-2 mt-1.5 mb-1 flex-none rounded border border-term-edge px-2 py-1 leading-[1.45]">
        <b>&gt;_ OpenAI Codex</b>
        <br />
        <span className="text-t-dim">model: gpt-5 medium · directory: ~/web</span>
      </div>
    ),
    prompt: <span>› </span>,
    user: "› ",
    say: "• ",
    tool: (text: string) => (
      <>
        • <span className="font-semibold">Ran</span> <span className="text-t-accent">{text}</span>
      </>
    ),
    result: "  └ ",
    work: "",
    input: "mt-1.5 bg-t-text/10 px-2.5 py-2",
    placeholder: "Ask Codex to do anything",
    foot: "gpt-5 medium · ~/web · main",
    shellFoot: (
      <>
        gpt-5 medium · ~/web · main<span className="float-right text-t-blocked">Shell mode</span>
      </>
    ),
  },
};

const isShell = (text: string) => text.startsWith("!");
const shellText = (text: string) => text.replace(/^!\s*/, "");

function item(value: Item, ui?: Agent): ReactNode {
  const look = ui && LOOK[ui];
  if (typeof value === "string") return value;
  if ("c" in value)
    return (
      <>
        <span className="text-t-accent">$ </span>
        {value.c}
      </>
    );
  if ("d" in value) return <span className="text-t-dim">{value.d}</span>;
  if ("w" in value) return <span className="text-t-waiting">{value.w}</span>;
  if ("x" in value) return <span className="text-t-blocked">{value.x}</span>;
  if ("o" in value) return <span className="text-t-working">{value.o}</span>;
  if ("u" in value)
    return (
      <span className="-mx-2.5 my-0.5 block bg-t-text/10 px-2.5 py-px">
        {isShell(value.u) ? (
          <>
            <span className="text-t-blocked">! </span>
            {shellText(value.u)}
          </>
        ) : (
          <>
            {look ? look.user : "> "}
            {value.u}
          </>
        )}
      </span>
    );
  if ("a" in value) return `${look ? look.say : ""}${value.a}`;
  if ("t" in value) return look ? look.tool(value.t) : value.t;
  if ("r" in value)
    return <span className="text-t-dim">{`${look ? look.result : "  "}${value.r}`}</span>;
  return <span className="text-t-dim">{`${look ? look.work : ""}${value.k}`}</span>;
}

function PaneBody({ pane }: { pane: PaneFrame }) {
  const cursor = pane.cursor && <span className="cursor"> </span>;
  const transcript = pane.lines.map((value, index) => (
    <div key={index} className="min-h-[1.55em]">
      {item(value, pane.ui)}
    </div>
  ));
  if (!pane.ui)
    return (
      <div className="flex min-h-0 flex-1 flex-col justify-end overflow-hidden px-2.5 py-1.5 break-words whitespace-pre-wrap">
        {transcript}
        {pane.typing !== undefined && (
          <div className="min-h-[1.55em]">
            <span className="text-t-accent">$ </span>
            {pane.typing}
            {cursor}
          </div>
        )}
      </div>
    );
  const look = LOOK[pane.ui];
  const text = pane.typing ?? "";
  const shell = isShell(text);
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden break-words whitespace-pre-wrap">
      {pane.lines.length === 0 && look.head}
      <div className="flex min-h-0 flex-1 flex-col justify-end overflow-hidden px-2.5">
        {transcript}
      </div>
      <div className={`flex-none ${look.input} ${shell ? "border-t-blocked" : ""}`}>
        {shell ? <span className="text-t-blocked">! </span> : look.prompt}
        {shell
          ? shellText(text)
          : text || (!pane.cursor && <span className="text-t-dim">{look.placeholder}</span>)}
        {cursor}
      </div>
      <div className="flex-none px-2.5 pt-px pb-1 text-[10.5px] text-t-dim">
        {shell ? look.shellFoot : look.foot}
      </div>
    </div>
  );
}

function BadgeLabel({ badge }: { badge?: Badge }) {
  if (!badge) return null;
  return (
    <b className={`font-semibold text-t-text ${badge.s === "ended" ? "opacity-50" : ""}`}>
      [{badge.s === "on" && <i className="text-t-working not-italic">● </i>}
      {badge.s === "ended" && <i className="text-t-dim not-italic">○ </i>}
      {badge.n} (tmt)]
    </b>
  );
}

function still(step: Step): Frame {
  const panes = step.panes.map((pane, index): PaneFrame => {
    const lines = pane.lines ?? [];
    const base = { ui: pane.ui, badge: pane.badge, lines, cursor: false };
    if (index !== step.focus) return base;
    if (pane.ui) return { ...base, typing: "", cursor: true };
    const last = lines.at(-1);
    if (last && typeof last === "object" && "c" in last && last.c === "")
      return { ...base, lines: lines.slice(0, -1), typing: "", cursor: true };
    return base;
  });
  return { focus: step.focus, panes };
}

const same = (a: Item, b: Item) => JSON.stringify(a) === JSON.stringify(b);
const jitter = () => 12 + Math.random() * 16;

export function PaneDemo({
  layout,
  steps,
  label,
}: {
  layout: Layout;
  steps: Step[];
  label: string;
}) {
  const root = useRef<HTMLDivElement>(null);
  const player = useStepPlayer<Frame>(
    {
      count: steps.length,
      still: (k) => still(steps[k]),
      hold: (k) => steps[k].ms ?? Math.max(1300, Math.min(2600, steps[k].cap.length * 16)),
      animate: async (k, emit, sleep) => {
        const step = steps[k];
        const previous = k > 0 ? steps[k - 1] : undefined;
        if (!previous) return true;
        const panes: PaneFrame[] = still(previous).panes.map((pane) => ({
          ...pane,
          typing: undefined,
          cursor: false,
        }));
        const show = () => emit({ focus: step.focus, panes: panes.map((pane) => ({ ...pane })) });
        show();
        const order = step.panes
          .map((_, index) => index)
          .sort((a, b) => Number(b === step.focus) - Number(a === step.focus));
        for (const index of order) {
          const target = step.panes[index];
          const now = target.lines ?? [];
          let was = previous.panes[index].lines ?? [];
          if (target.ui !== previous.panes[index].ui) {
            if (target.launch) {
              panes[index] = { lines: [], typing: "", cursor: true };
              for (let c = 0; c <= target.launch.length; c++) {
                panes[index].typing = target.launch.slice(0, c);
                show();
                if (!(await sleep(c === 0 ? 160 : jitter()))) return false;
              }
              if (!(await sleep(220))) return false;
            } else if (!(await sleep(120))) return false;
            panes[index] = { ui: target.ui, badge: panes[index].badge, lines: [], cursor: false };
            was = [];
            show();
            if (!(await sleep(260))) return false;
          }
          let common = 0;
          while (common < was.length && common < now.length && same(was[common], now[common]))
            common++;
          if (common === now.length && common === was.length) {
            panes[index].badge = target.badge;
            continue;
          }
          panes[index].lines = now.slice(0, common);
          show();
          for (let j = common; j < now.length; j++) {
            const next = now[j];
            const typed =
              typeof next === "object" && ("c" in next ? next.c : "u" in next ? next.u : undefined);
            if (typed) {
              for (let c = 0; c <= typed.length; c++) {
                panes[index].typing = typed.slice(0, c);
                panes[index].cursor = true;
                show();
                if (!(await sleep(c === 0 ? 160 : jitter()))) return false;
              }
              if (!(await sleep(140))) return false;
            } else if (!(await sleep(75))) return false;
            panes[index].typing = undefined;
            panes[index].cursor = false;
            panes[index].lines = now.slice(0, j + 1);
            show();
          }
          panes[index].badge = target.badge;
        }
        return true;
      },
    },
    root,
  );
  const step = steps[player.index];
  return (
    <div ref={root} className="my-5 w-full" role="group" aria-label={label}>
      <div aria-hidden="true" className="overflow-hidden rounded-md bg-term">
        <FitWidth width={640}>
          <div className="grid h-[400px] grid-cols-2 grid-rows-2 font-mono text-xs leading-[1.55] text-t-text">
            {layout.map((pane, index) => {
              const frame = player.frame.panes[index];
              const focused = player.frame.focus === index;
              return (
                <div
                  key={pane.id}
                  className={`relative flex min-w-0 flex-col overflow-hidden border ${pane.tall ? "row-span-2" : ""} ${
                    focused ? "border-t-accent" : "border-term-edge"
                  }`}
                >
                  <div
                    className={`flex items-center gap-1.5 border-b px-2 py-[3px] text-[11px] whitespace-nowrap text-t-dim ${
                      focused ? "border-t-accent" : "border-term-edge"
                    }`}
                  >
                    <span>{pane.id}</span>
                    <BadgeLabel badge={frame?.badge} />
                  </div>
                  {frame && <PaneBody pane={frame} />}
                </div>
              );
            })}
          </div>
        </FitWidth>
      </div>
      <DemoControls
        caption={step.cap}
        count={steps.length}
        index={player.index}
        playing={player.playing}
        prev={player.prev}
        next={player.next}
        toggle={player.toggle}
      />
    </div>
  );
}

const R = "reviewer";
const B = "builder";
const threePanes: Layout = [{ id: "0.1", tall: true }, { id: "0.2" }, { id: "0.3" }];

function serverLog(failing: boolean): Item[] {
  const lines: Item[] = [
    { d: "> web@1.4 dev" },
    "ready on :3000",
    "GET /login 200 12ms",
    "POST /login 200 31ms",
  ];
  return failing
    ? [
        ...lines,
        "POST /login 500 8ms",
        { x: "TypeError: session is undefined" },
        { x: "    at createSession (auth.ts:88)" },
      ]
    : lines;
}
const log0: Pane = { lines: serverLog(false) };
const logNamed: Pane = { badge: { n: "server-log", s: "none" }, lines: serverLog(false) };
const logFailing: Pane = { badge: { n: "server-log", s: "none" }, lines: serverLog(true) };
const shell: Pane = { lines: [{ c: "" }] };
const a1: Pane = { ui: "claude", launch: "tmt run claude", lines: [] };
const b1: Pane = { ui: "codex", launch: "tmt run codex", lines: [] };
const a2: Pane = {
  ui: "claude",
  badge: { n: R, s: "on" },
  lines: [{ u: "!tmt name reviewer" }, { r: "✓ Bound temporary identity 'reviewer' on pane %0" }],
};
const b2: Pane = {
  ui: "codex",
  badge: { n: B, s: "on" },
  lines: [
    { u: "!tmt name builder" },
    { r: "✓ Bound temporary identity 'builder' on pane %1" },
    { u: "!tmt add 0.3 server-log" },
    { r: "✓ Bound temporary identity 'server-log' on pane %2" },
  ],
};
const a3: Pane = {
  ...a2,
  lines: [
    ...a2.lines!,
    { u: "Ask builder to fix the flaky login test." },
    { t: 'tmt talk builder "Fix the flaky login test"' },
  ],
};
const b3lines: Item[] = [
  { d: '<tmt-reply from="reviewer">' },
  "Fix the flaky login test",
  { a: "The redirect isn’t awaited. Fixing." },
  { t: "npm test auth.spec.ts" },
  { r: "20 passed" },
];
const b4: Pane = {
  ui: "codex",
  badge: { n: B, s: "on" },
  lines: [
    ...b3lines,
    { t: 'tmt reply req_4f36… --receipt v2_… --message "Fixed"' },
    { r: "✓ Submitted response for request req_4f36…" },
    { k: "Worked for 41s" },
  ],
};
const a4: Pane = {
  ...a2,
  lines: [
    ...a3.lines!,
    { r: "✓ Completed request req_4f36… for builder (%1)" },
    { r: "Fixed" },
    { a: "builder fixed it: the redirect is now" },
    { d: "  awaited, and 20/20 runs pass." },
    { k: "Churned for 48s" },
  ],
};
const a5: Pane = {
  ...a2,
  lines: [
    { a: "builder fixed it: 20/20 runs pass." },
    { u: "Login still 500s. Check server-log." },
    { t: "tmt check server-log 20" },
    { r: "TypeError: session is undefined" },
    { r: "  at createSession (auth.ts:88)" },
    { a: "Found it: auth.ts:88 reads the session" },
    { d: "  before the cookie is parsed. Fixing." },
  ],
};

export const basicLoop = {
  layout: threePanes,
  steps: [
    { focus: 0, panes: [shell, shell, log0], cap: "Two empty panes and your dev server’s log." },
    {
      focus: 0,
      panes: [a1, shell, log0],
      cap: "Start Claude Code through tmt. It opens as usual.",
    },
    {
      focus: 1,
      panes: [a1, b1, log0],
      cap: "Same for Codex in the next pane. Two agents, no names yet.",
    },
    {
      focus: 0,
      panes: [a2, b1, log0],
      cap: "Inside Claude Code, press ! for shell mode and run tmt name reviewer. The pane gets its name and badge.",
    },
    {
      focus: 1,
      panes: [a2, b2, logNamed],
      cap: "Same in Codex: ! tmt name builder, then ! tmt add 0.3 server-log names the log pane. Any pane can have a name.",
    },
    {
      focus: 0,
      panes: [a3, b2, logNamed],
      cap: "Ask reviewer for something. It talks to builder on its own.",
    },
    {
      focus: 1,
      panes: [a3, b4, logNamed],
      cap: "builder gets the request, does the work and answers with tmt reply.",
    },
    {
      focus: 0,
      panes: [a4, b4, logFailing],
      cap: "The reply lands in reviewer’s session. Meanwhile the server starts throwing.",
    },
    {
      focus: 0,
      panes: [a5, b4, logFailing],
      cap: "Tell reviewer to check server-log. It reads that pane by name and finds the bug.",
    },
  ] satisfies Step[],
};

const conversationR: Item[] = [
  { u: "Audit token rotation." },
  { a: "Reading auth/tokens.ts…" },
  { k: "Churned for 2m 10s" },
];
const conversationB: Item[] = [
  { u: "Run the test suite." },
  { t: "npm test" },
  { r: "312 passed" },
  { k: "Worked for 1m 4s" },
];
const rv: Pane = { ui: "claude", badge: { n: R, s: "on" }, lines: conversationR };
const bd: Pane = { ui: "codex", badge: { n: B, s: "on" }, lines: conversationB };
const bare: Pane = { lines: [{ c: "" }] };
const bareR: Pane = { badge: { n: R, s: "ended" }, lines: [{ c: "" }] };
const bareB: Pane = { badge: { n: B, s: "ended" }, lines: [{ c: "" }] };
const rv2: Pane = { ...rv, launch: "tmt resume reviewer" };
const bd2: Pane = { ...bd, launch: "tmt resume builder" };
const missing: Pane = {
  lines: [
    { c: "tmt resume helper" },
    { w: "helper’s conversation no longer exists." },
    { w: "Start fresh with: tmt run helper" },
  ],
};

export const resumeAfterReboot = {
  layout: threePanes,
  steps: [
    {
      focus: 0,
      panes: [rv, bd, bare],
      cap: "Before: reviewer (Claude Code) and builder (Codex) are working, each in its own conversation.",
    },
    {
      focus: 0,
      panes: [bareR, bareB, bare],
      cap: "After a reboot, your tmux layout is back but the agents aren’t. tmt still remembers each name’s conversation and model.",
    },
    {
      focus: 0,
      panes: [rv2, bareB, bare],
      cap: "tmt resume reviewer reopens the same Claude Code conversation, with the same model.",
    },
    {
      focus: 1,
      panes: [rv2, bd2, bare],
      cap: "Same for builder: the Codex driver reopens its own thread.",
    },
    {
      focus: 2,
      panes: [rv2, bd2, missing],
      cap: "If a conversation is gone, tmt says so and doesn’t start a fresh one behind your back.",
    },
  ] satisfies Step[],
};
