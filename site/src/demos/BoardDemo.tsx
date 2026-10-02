import { useRef } from "react";
import type { ReactNode } from "react";
import { boardLines, type BoardSpec, type Row } from "./board";
import { FitWidth } from "./FitWidth";
import { DemoControls, useStepPlayer } from "./player";
import { dm, line, sh, wt, type Line } from "./segments";

// A tmux screen with a shell or agent pane, and the squad board as a popup or
// full screen, typed out step by step.
type Step = {
  cap: string;
  sess: string;
  windows: string[];
  on: number;
  right: string;
  pre?: Line[];
  prompt?: string;
  type?: string;
  out?: Line[];
  pops?: BoardSpec[];
  full?: boolean;
  toast?: string;
  hold?: number;
};

type Frame = {
  pane: Line[];
  typed?: { prompt: string; text: string; cursor: boolean; out: number };
  pop?: { board: BoardSpec; typed: string; replies: number; cursor: boolean };
  toast?: string;
};

function still(step: Step): Frame {
  const last = step.pops?.at(-1);
  return {
    pane: step.pre ?? [],
    typed:
      step.type === undefined
        ? undefined
        : {
            prompt: step.prompt ?? "$ ",
            text: step.type,
            cursor: false,
            out: step.out?.length ?? 0,
          },
    pop: last && {
      board: last,
      typed: last.typed ?? "",
      replies: last.reply?.length ?? 0,
      cursor: false,
    },
    toast: step.toast,
  };
}

const jitter = () => 14 + Math.random() * 14;

export function BoardDemo({ steps, label }: { steps: Step[]; label: string }) {
  const root = useRef<HTMLDivElement>(null);
  const player = useStepPlayer<Frame>(
    {
      count: steps.length,
      still: (k) => still(steps[k]),
      hold: (k) => steps[k].hold ?? 1800,
      animate: async (k, emit, sleep) => {
        const step = steps[k];
        const base: Frame = { pane: step.pre ?? [] };
        const prompt = step.prompt ?? "$ ";
        if (step.type !== undefined) {
          emit({ ...base, typed: { prompt, text: "", cursor: true, out: 0 } });
          if (!(await sleep(200))) return false;
          for (let c = 1; c <= step.type.length; c++) {
            emit({ ...base, typed: { prompt, text: step.type.slice(0, c), cursor: true, out: 0 } });
            if (!(await sleep(jitter()))) return false;
          }
          if (!(await sleep(150))) return false;
          for (let o = 1; o <= (step.out?.length ?? 0); o++) {
            emit({ ...base, typed: { prompt, text: step.type, cursor: false, out: o } });
            if (!(await sleep(110))) return false;
          }
        }
        const typed =
          step.type === undefined
            ? undefined
            : { prompt, text: step.type, cursor: false, out: step.out?.length ?? 0 };
        const pops = step.pops ?? [];
        for (let p = 0; p < pops.length; p++) {
          const board = pops[p];
          if ((p > 0 || step.type !== undefined) && !(await sleep(p > 0 ? 600 : 200))) return false;
          if (board.typed) {
            emit({ ...base, typed, pop: { board, typed: "", replies: 0, cursor: true } });
            if (!(await sleep(250))) return false;
            for (let c = 1; c <= board.typed.length; c++) {
              emit({
                ...base,
                typed,
                pop: { board, typed: board.typed.slice(0, c), replies: 0, cursor: true },
              });
              if (!(await sleep(jitter()))) return false;
            }
            if (!(await sleep(200))) return false;
            for (let r = 1; r <= (board.reply?.length ?? 0); r++) {
              emit({
                ...base,
                typed,
                pop: { board, typed: board.typed, replies: r, cursor: false },
              });
              if (!(await sleep(r === 1 ? 250 : 450))) return false;
            }
          } else {
            emit({ ...base, typed, pop: { board, typed: "", replies: 0, cursor: false } });
          }
        }
        if (step.toast && !(await sleep(400))) return false;
        return true;
      },
    },
    root,
  );
  const step = steps[player.index];
  const { frame } = player;
  const paneLines: ReactNode[] = frame.pane.map((value, index) => (
    <div key={index}>{line(value)}</div>
  ));
  if (frame.typed) {
    paneLines.push(
      <div key="typed">
        <span className="text-t-accent">{frame.typed.prompt}</span>
        {frame.typed.text}
        {frame.typed.cursor && <span className="cursor"> </span>}
      </div>,
    );
    (step.out ?? [])
      .slice(0, frame.typed.out)
      .forEach((value, index) => paneLines.push(<div key={`o${index}`}>{line(value)}</div>));
  }
  return (
    <div ref={root} className="my-6 w-full" aria-label={label} role="group">
      <div
        aria-hidden="true"
        className="overflow-hidden border border-term-edge bg-term shadow-[6px_6px_0_var(--c-accent)]"
      >
        <FitWidth width={620}>
          <div className="relative h-[340px] overflow-hidden font-mono text-[12.5px] leading-[1.6] text-t-text">
            <div className="absolute inset-0 flex flex-col justify-end overflow-hidden px-3.5 py-2.5 whitespace-pre">
              {paneLines}
            </div>
            <div
              className={`absolute overflow-hidden bg-term px-3 py-2 whitespace-pre transition-[opacity,transform] duration-200 motion-reduce:transition-none ${
                step.full
                  ? "inset-0"
                  : "inset-[5%] border border-t-accent shadow-[0_10px_40px_rgba(0,0,0,.45)]"
              } ${frame.pop ? "opacity-100" : "pointer-events-none translate-y-1.5 scale-[.985] opacity-0"}`}
            >
              {frame.pop &&
                boardLines(frame.pop.board, frame.pop.typed, frame.pop.replies, frame.pop.cursor)}
            </div>
            <div
              className={`absolute right-3 bottom-3 bg-t-accent px-2.5 py-2 text-xs leading-none font-semibold text-term transition-opacity ${
                frame.toast ? "opacity-100" : "opacity-0"
              }`}
            >
              {frame.toast}
            </div>
          </div>
        </FitWidth>
      </div>
      <div
        aria-hidden="true"
        className="flex items-center gap-3 overflow-hidden bg-term-bar px-2.5 py-1.5 font-mono text-xs leading-none font-medium whitespace-nowrap text-t-dim"
      >
        <span className="font-bold text-t-accent">[{step.sess}]</span>
        {step.windows.map((window, k) => (
          <span
            key={window}
            className={
              k === step.on
                ? "font-bold text-t-text underline decoration-t-accent underline-offset-4"
                : ""
            }
          >
            {window}
          </span>
        ))}
        <span className="ml-auto">{step.right}</span>
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

// Demo 1: start a squad, talk to the lead from the board.
const s1: Line[] = [sh("tmt sq init product"), dm("✓ Created squad product (room squad-product)")];
const s2: Line[] = [...s1, sh("tmt sq lead sol"), dm("✓ sol leads squad product")];
const W1 = ["0:shell", "1:sol"];

export const startSquad: Step[] = [
  {
    cap: "Create a squad. Under the hood it is a meeting room, squad-product.",
    sess: "leads",
    windows: W1,
    on: 0,
    right: "squad-product",
    type: "tmt sq init product",
    out: [dm("✓ Created squad product (room squad-product)")],
  },
  {
    cap: "Make sol, the agent in window 1, its lead. sol joins the room and keeps the board.",
    sess: "leads",
    windows: W1,
    on: 0,
    right: "squad-product",
    pre: s1,
    type: "tmt sq lead sol",
    out: [dm("✓ sol leads squad product")],
  },
  {
    cap: "tmt sq opens the board full screen in your pane. Tell your lead what you need; the reply arrives right on the board.",
    sess: "leads",
    windows: W1,
    on: 0,
    right: "squad-product",
    pre: s2,
    type: "tmt sq",
    out: [],
    full: true,
    hold: 2200,
    pops: [
      {
        rows: [],
        to: "sol",
        typed: "Start two members: token rotation, and one install guide.",
        reply: [dm("  waiting for sol…"), "  sol: On it. I’ll set up a crew and report back."],
      },
    ],
  },
  {
    cap: "sol sets up the crew its own way: a crew session, a worktree and window per member, then dispatches. The board fills in as it goes.",
    sess: "leads",
    windows: W1,
    on: 0,
    right: "squad-product · crew 2",
    pre: s2,
    full: true,
    hold: 3000,
    pops: [
      { rows: [["auth-fix", "starting", "rotate session tokens", ""]] },
      {
        rows: [
          ["auth-fix", "working", "rotate session tokens", ""],
          ["docs-sweep", "starting", "one install guide", ""],
        ],
      },
      {
        rows: [
          ["auth-fix", "working", "rotate session tokens", "#412 draft"],
          ["docs-sweep", "working", "one install guide", ""],
        ],
        detail: "sol: two members working · nothing waiting on you",
      },
    ],
  },
];

// Demo 2: jump, jump back, and talk from the board.
const WL = ["0:shell", "1:sol"];
const WC = ["1:auth-fix", "2:docs-sweep"];
const blocked: Row[] = [
  ["auth-fix", "blocked", "rotate session tokens", "#412 draft", true],
  ["docs-sweep", "working", "one install guide", "#409 open"],
];
const working: Row[] = [
  ["auth-fix", "working", "rotate session tokens", "#412 draft"],
  ["docs-sweep", "working", "one install guide", "#409 open"],
];
const agent: Line[] = [
  dm("auth-fix · ~/w/app-3 · fix/token"),
  "",
  "● Two ways to rotate session tokens:",
  "  1. rotate on next login  (simple, slower rollout)",
  "  2. background sweep     (fast, needs a new job)",
  "",
  wt("◆ waiting for your call"),
];

export const jumpAndTalk: Step[] = [
  {
    cap: "Here the board is a tmux popup (prefix S, if you installed the hotkeys). auth-fix is marked ◆: it is waiting on you, so it sorts first.",
    sess: "leads",
    windows: WL,
    on: 1,
    right: "squad-product · crew 2",
    pre: s2,
    pops: [
      {
        rows: blocked,
        sel: 0,
        pending: "approve the token rotation plan",
        detail: "auth-fix · ~/w/app-3 · fix/token · ● running",
      },
    ],
  },
  {
    cap: "Enter jumps to the member’s own screen, even in another session. Answer it there.",
    sess: "crew",
    windows: WC,
    on: 0,
    right: "squad-product · crew 2",
    pre: agent,
    prompt: "> ",
    type: "go with 1",
    out: ["", "● On it. Rotating on next login."],
  },
  {
    cap: "Open the board again and press ⌫ to jump back to where you were.",
    sess: "crew",
    windows: WC,
    on: 0,
    right: "squad-product · crew 2",
    pre: [...agent, "> go with 1", "", "● On it. Rotating on next login."],
    hold: 1500,
    pops: [{ rows: working, sel: 0, detail: "⌫ back to leads:1 sol" }],
  },
  {
    cap: "Back in leads, exactly where you left off.",
    sess: "leads",
    windows: WL,
    on: 1,
    right: "squad-product · crew 2",
    pre: s2,
    hold: 1600,
  },
  {
    cap: "Or never leave the board: t sends a prompt to the selected member, and its reply shows up right here.",
    sess: "leads",
    windows: WL,
    on: 1,
    right: "squad-product · crew 2",
    pre: s2,
    hold: 3000,
    pops: [
      {
        rows: working,
        sel: 1,
        detail: "docs-sweep · ~/w/app-1 · docs/install · ● running",
        to: "docs-sweep",
        typed: "Is the guide ready for review?",
        reply: [
          dm("  waiting for docs-sweep…"),
          "  docs-sweep: Yes. One page, three platform tabs. PR #409 is open.",
        ],
      },
    ],
  },
  {
    cap: "y copies the entry as a status line for your own update.",
    sess: "leads",
    windows: WL,
    on: 1,
    right: "squad-product · crew 2",
    pre: s2,
    hold: 2600,
    pops: [{ rows: working, sel: 1, detail: "docs-sweep · ~/w/app-1 · docs/install · ● running" }],
    toast: "copied · docs-sweep: one install guide (working) #409",
  },
];
