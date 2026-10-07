import type { ReactNode } from "react";
import { dm, line, pad, type Line } from "./segments";
import { FitWidth } from "./FitWidth";

// One board frame in the squad walkthroughs.
export type Row = [name: string, state: string, task: string, pr: string, waiting?: boolean];
export type BoardSpec = {
  rows: Row[];
  sel?: number | "lead";
  expanded?: boolean;
  to?: string;
  mode?: "note" | "talk";
  typed?: string;
  reply?: Line[];
  status?: { lines: { text: string; field?: number }[]; focus: number };
};

export type StatusStage =
  | "initial"
  | "chosen"
  | "conflict"
  | "fresh"
  | "failed"
  | "retry"
  | "accepted"
  | "unknown";

// Illustrative frames, not mutations or native fixture output.
export function statusBoard(stage: StatusStage): BoardSpec {
  const checked = stage !== "initial" && stage !== "conflict";
  const applied = ["failed", "retry", "accepted"].includes(stage);
  const oldState = ["conflict", "fresh", "failed", "retry", "accepted"].includes(stage)
    ? "review"
    : "blocked";
  const fields = [
    { text: `[${checked ? "x" : " "}] Clear pending`, field: 0 },
    { text: `  Old: " approve plan " → New: ${checked ? "(empty)" : "(unchanged)"}`, field: 0 },
    {
      text: `[${checked ? "x" : " "}] Replace state: ${stage === "initial" ? "" : "working"}`,
      field: 1,
    },
    { text: `  Old: "${oldState}" → New: ${checked ? '"working"' : "(unchanged)"}`, field: 1 },
    { text: `Reason: ${stage === "initial" ? "" : "Approval recorded"}`, field: 2 },
    {
      text:
        stage === "failed" || stage === "retry"
          ? "Retry notification only"
          : stage === "accepted"
            ? "Status applied"
            : stage === "unknown"
              ? "Outcome unknown — reopen only after"
              : "Apply and notify",
      field: 3,
    },
  ];
  if (stage === "unknown") fields.push({ text: "inspecting metadata", field: 3 });
  const notices: Partial<Record<StatusStage, string[]>> = {
    conflict: ["Selected values changed; preview refreshed.", "Review and submit again."],
    failed: ["Status updated; notification failed. …"],
    retry: ["Status updated; notification failed. …"],
    accepted: ["Status updated; notification accepted", "(queued, no reply required)."],
    unknown: ["Apply outcome unknown; no notification sent", "or replay permitted. …"],
  };
  const lines: { text: string; field?: number }[] = [
    ...fields,
    ...(notices[stage] ?? []).map((text) => ({ text })),
  ];
  if (!applied && stage !== "unknown")
    lines.push(
      { text: "Requires a reply (separate from manual status)" },
      { text: "Answer request req_11111111-1111-", field: 4 },
      { text: "4111-8111-111111111111: Ship tonight?", field: 4 },
      { text: "Answer request req_22222222-2222-", field: 5 },
      { text: "4222-8222-222222222222: Choose API shape?", field: 5 },
    );
  return {
    rows: [
      ["auth-fix", applied ? "working" : oldState, "rotate session tokens", "#412 draft", true],
      ["docs-sweep", "working", "one install guide", "#409 open"],
    ],
    sel: 0,
    status: {
      lines,
      focus: stage === "chosen" ? 2 : ["fresh", "retry", "unknown"].includes(stage) ? 3 : 0,
    },
  };
}

export function boardLines(
  board: BoardSpec,
  typed: string,
  replies: number,
  cursor: boolean,
): ReactNode[] {
  const lines: ReactNode[] = [];
  const push = (value: ReactNode) =>
    lines.push(<div key={lines.length}>{value === "" ? " " : value}</div>);
  push(
    <>
      <span className="font-bold text-t-accent">squad</span>
      {"  [product]                    "}
      {line(
        dm("lead sol · " + board.rows.length + (board.rows.length === 1 ? " member" : " members")),
      )}
    </>,
  );
  push("");
  push(line(dm("┌" + "─".repeat(50) + "┐")));
  const leadSelected = board.sel === "lead";
  push(
    <span className={leadSelected ? "bg-t-selection" : undefined}>
      <span className="text-t-dim">│ </span>
      {leadSelected ? "▸ " : "  "}
      <span className="font-bold">sol</span>
      {"  "}
      <span className="text-t-dim">lead</span>
      {"  "}
      <span className="text-t-working">working</span>
    </span>,
  );
  const pushBand = (task: string, pr?: string) => {
    push(<span className="bg-t-selection">{"│    task · " + task}</span>);
    if (pr) push(<span className="bg-t-selection">{"│    links · " + pr}</span>);
    if (replies > 0 && board.reply) {
      push(
        <span className="bg-t-selection">
          {"│    latest reply · "}
          {line(board.reply[replies - 1])}
        </span>,
      );
    }
    push(<span className="bg-t-selection text-t-dim">{"│    e collapse  a write  o open"}</span>);
  };
  if (leadSelected && board.expanded) pushBand("coordinate the squad");
  else push(line(dm("│   coordinate the squad")));
  const pushComposer = () => {
    if (board.status) {
      push(<span className="text-t-accent">{"│ Update status → product / auth-fix"}</span>);
      board.status.lines.forEach(({ text, field }) => {
        const focused = field === board.status?.focus;
        push(
          <span className={focused ? "text-t-accent" : undefined}>
            {"│ " + (field === undefined ? "" : focused ? "› " : "  ") + text}
          </span>,
        );
      });
      push(line(dm("│ ↑↓ field · Enter choose · PgUp/PgDn")));
      push(line(dm("│ Esc cancel · Tab answer/note/talk/status")));
      return;
    }
    if (!board.to) return;
    push(
      <span className="text-t-accent">
        {"│ → " + board.to + " (product) · " + (board.mode ?? "note")}
      </span>,
    );
    const draftLines = typed.match(/.{1,48}/g) ?? [""];
    draftLines.forEach((text, index) =>
      push(
        <span>
          {"│ "}
          {text}
          {cursor && index === draftLines.length - 1 && <span className="cursor"> </span>}
        </span>,
      ),
    );
    push(line(dm("│ Enter send · Esc cancel · Tab note/talk/status")));
  };
  if (leadSelected) pushComposer();
  push(line(dm("│ ── members · " + board.rows.length + " ──")));
  if (board.rows.length) {
    board.rows.forEach((row, index) => {
      const [name, state, task, pr, waiting] = row;
      const tone =
        state === "working"
          ? "text-t-working"
          : state === "blocked"
            ? "text-t-waiting"
            : "text-t-dim";
      const selected = index === board.sel;
      const content = (
        <>
          <span className="text-t-dim">│ </span>
          {selected ? "▸ " : "  "}
          {waiting ? <span className="text-t-waiting">◆ </span> : "  "}
          <span className="font-bold">{pad(name, 13)}</span>
          <span className={tone}>{state}</span>
        </>
      );
      push(selected ? <span className="bg-t-selection">{content} </span> : content);
      if (selected && board.expanded) pushBand(task, pr);
      else
        push(
          <span className={selected ? "bg-t-selection" : "text-t-dim"}>
            {"│    "}
            {task}
            {pr ? " · " + pr : ""}
          </span>,
        );
      if (selected) pushComposer();
    });
  } else {
    push(line(dm("│   no members yet")));
  }
  push(line(dm("└" + "─".repeat(50) + "┘")));
  push("");
  push(line(dm("↑↓ move  ⏎ open  a write  e expand  A ask lead  / search  ? more  q quit")));
  return lines;
}

const keys: [string, string][] = [
  ["↑↓", "move"],
  ["⏎", "open"],
  ["a", "write"],
  ["e", "expand"],
  ["A", "ask lead"],
  ["/", "search"],
  ["?", "more"],
  ["q", "quit"],
];

function KeyBar() {
  return (
    <div className="term-scroll bg-term-bar px-3.5 py-2 font-mono text-xs leading-none whitespace-nowrap text-t-dim">
      {keys.map(([key, action]) => (
        <span key={key} className="mr-3.5">
          <b className="mr-1 font-semibold text-t-accent">{key}</b>
          {action}
        </span>
      ))}
    </div>
  );
}

const sketch = "my-6 w-full overflow-hidden bg-term text-t-text";
const pre = "term-scroll m-0 px-3.5 pt-3 pb-3.5 font-mono text-[12.5px] leading-[1.6]";

export function BoardSketch() {
  return (
    <div
      role="img"
      aria-label="Sketch of the default members view: one boxed list with lead sol first, then four members. Auth-fix is selected and waiting on you. Each row has a task preview; the lead's notes sit below the box. The footer includes a to write, e to expand and question mark for more actions."
      className={sketch}
    >
      <pre className={pre}>
        <span className="bg-t-selection font-bold text-t-waiting">{" product ◆1 ✗1 "}</span>
        <span className="text-t-muted">{" reviews  infra "}</span>
        {"\n"}
        <span className="text-t-muted">lead sol · 4 members</span>
      </pre>
      <div className="mx-3.5 mb-3.5 border border-term-edge font-mono text-[12.5px] leading-[1.6]">
        <div className="border-b border-term-edge px-3 py-2">
          <div className="flex flex-wrap gap-x-3">
            <span className="font-bold">sol</span>
            <span className="text-t-dim">lead</span>
            <span className="text-t-working">working</span>
            <span className="ml-auto text-t-muted">sol · 2m</span>
          </div>
          <div className="text-t-dim">coordinate the release</div>
        </div>
        <div className="border-b border-term-edge px-3 text-t-muted">── members · 4 ──</div>
        <div className="border-b border-term-edge bg-t-selection px-3 py-2">
          <div className="flex flex-wrap gap-x-3">
            <span className="font-bold text-t-waiting">◆ auth-fix</span>
            <span className="text-t-blocked">blocked</span>
            <span className="ml-auto text-t-muted">sol · 5m</span>
          </div>
          <div>approve rotation plan</div>
        </div>
        <div className="border-b border-term-edge px-3 py-2">
          <div className="flex flex-wrap gap-x-3">
            <span className="font-bold">docs-sweep</span>
            <span className="text-t-review">review</span>
            <span className="ml-auto text-t-muted">luna · 8m</span>
          </div>
          <div className="text-t-dim">update install guide</div>
        </div>
        <div className="border-b border-term-edge px-3 py-2">
          <div className="flex flex-wrap gap-x-3">
            <span className="font-bold">perf-cache</span>
            <span className="text-t-working">working</span>
            <span className="ml-auto text-t-muted">sol · 12m</span>
          </div>
          <div className="text-t-dim">cache reads</div>
        </div>
        <div className="px-3 py-2 text-t-dim">
          <div className="flex flex-wrap gap-x-3">
            <span className="font-bold">old-spike</span>
            <span>idle</span>
            <span className="ml-auto">3h</span>
          </div>
          <div>parser spike</div>
        </div>
      </div>
      <pre className={pre}>
        <span className="text-t-muted">notes · sol</span>
        {"\n"}
        <span className="font-bold text-t-accent">## Now</span>
        {"\n- tokens: waiting on Ben's call\n- install guide: one page, platform tabs"}
      </pre>
      <KeyBar />
    </div>
  );
}

export function SplitBoardSketch() {
  return (
    <div
      role="img"
      aria-label="Sketch of a split board: rows on the left with auth-fix selected, showing what the member waits on you for and an annotation sent to sol; an opaque note composer directly below the complete selected row and before docs-sweep and perf-cache, spanning the split panes; and the lead's notes on the right."
      className={sketch}
    >
      <div className="grid grid-cols-1 sm:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
        <pre className={`${pre} border-b border-term-edge sm:border-r sm:border-b-0`}>
          <span className="font-bold text-t-accent">squad</span>
          {"  [product]      "}
          <span className="text-t-dim">rows</span>
          {"\n\n"}
          <span className="text-t-dim">{"    MEMBER      STATE    PR"}</span>
          {"\n"}
          <span className="bg-t-selection">
            {"▸ "}
            <span className="text-t-waiting">◆</span>
            {" auth-fix    "}
            <span className="text-t-waiting">blocked</span>
            {"  #412 "}
          </span>
          {"\n      "}
          <span className="text-t-dim">pending</span>
          {" needs login-vs-sweep call\n      "}
          <span className="text-t-waiting">✎ sent to sol</span>
          {" keep old tokens…"}
        </pre>
        <pre className={`${pre} hidden sm:block`}>
          <span className="text-t-dim">notes · sol</span>
          {"\n\n"}
          <span className="font-bold text-t-accent">## Now</span>
          {"\n- tokens: waiting on Ben's\n  call (login vs sweep)"}
        </pre>
        <div className="term-scroll border-y border-term-edge bg-term-bar px-3.5 py-2 font-mono text-[12.5px] leading-normal whitespace-pre sm:col-span-2">
          <b className="font-semibold text-t-accent">→ sol (product) · note · about auth-fix</b>
          {"\nkeep old tokens valid for a day"}
          <span className="inline-block w-[0.6em] bg-t-text"> </span>
          {"\nEnter send · Esc cancel · Tab note/talk/status"}
        </div>
        <pre className={`${pre} border-b border-term-edge sm:border-r sm:border-b-0`}>
          {"    docs-sweep  "}
          <span className="text-t-working">working</span>
          {"  #409\n    perf-cache  "}
          <span className="text-t-working">working</span>
          {"  "}
          <span className="text-t-dim">–</span>
        </pre>
        <pre className={pre}>
          <span className="sm:hidden">
            <span className="text-t-dim">notes · sol</span>
            {"\n\n"}
            <span className="font-bold text-t-accent">## Now</span>
            {"\n- tokens: waiting on Ben's\n  call (login vs sweep)\n"}
          </span>
          {"- install guide: one page,\n  platform tabs\n\n"}
          <span className="font-bold text-t-accent">## Decided</span>
          {"\n- no new job runner this\n  quarter"}
        </pre>
      </div>
    </div>
  );
}

export function UpdateStatusSketch() {
  return (
    <div
      role="img"
      aria-label="Update status sketch: the complete auth-fix row and task precede the inline band. Both manual changes are checked, but Reason has the non-color focus cue. Two unanswered requests remain separate; docs-sweep follows the band."
      className={sketch}
    >
      <FitWidth width={620}>
        <div className="px-3.5 py-3 font-mono text-[12.5px] leading-[1.6] whitespace-pre">
          {boardLines(statusBoard("chosen"), "", 0, false)}
        </div>
      </FitWidth>
    </div>
  );
}
