import type { ReactNode } from "react";
import { dm, line, pad, type Line } from "./segments";

// One board frame in the squad walkthroughs.
export type Row = [name: string, state: string, task: string, pr: string, waiting?: boolean];
export type BoardSpec = {
  rows: Row[];
  sel?: number | "lead";
  expanded?: boolean;
  to?: string;
  typed?: string;
  reply?: Line[];
};

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
    });
  } else {
    push(line(dm("│   no members yet")));
  }
  push(line(dm("└" + "─".repeat(50) + "┘")));
  if (board.to) {
    push(
      <>
        <span className="text-t-accent">t ▸ {board.to}</span>
        {"  "}
        {typed}
        {cursor && <span className="cursor"> </span>}
      </>,
    );
  }
  push("");
  push(line(dm("⏎ jump  ⌫ back  t talk  r reply  e expand  o open  y copy  n notes  l view")));
  return lines;
}

const keys: [string, string][] = [
  ["⏎", "jump"],
  ["⌫", "back"],
  ["/", "search"],
  ["t", "talk"],
  ["r", "reply"],
  ["e", "expand"],
  ["a", "annotate"],
  ["o", "open"],
  ["y", "copy"],
  ["n", "notes"],
  ["l", "view"],
  ["?", "more"],
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
      aria-label="Sketch of the default members view: one boxed list with lead sol first, then four members. Auth-fix is selected and waiting on you. Each row has a task preview; the lead's notes sit below the box. The key legend includes e to expand a row, n for notes and l for views."
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
      aria-label="Sketch of a split board: rows on the left with auth-fix selected, showing what the member waits on you for and an annotation sent to sol; the lead's notes on the right; and an annotation being typed."
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
          {" keep old tokens…\n    docs-sweep  "}
          <span className="text-t-working">working</span>
          {"  #409\n    perf-cache  "}
          <span className="text-t-working">working</span>
          {"  "}
          <span className="text-t-dim">–</span>
        </pre>
        <pre className={pre}>
          <span className="text-t-dim">notes · sol</span>
          {"\n\n"}
          <span className="font-bold text-t-accent">## Now</span>
          {
            "\n- tokens: waiting on Ben's\n  call (login vs sweep)\n- install guide: one page,\n  platform tabs\n\n"
          }
          <span className="font-bold text-t-accent">## Decided</span>
          {"\n- no new job runner this\n  quarter"}
        </pre>
      </div>
      <div className="term-scroll border-t border-term-edge bg-term-bar px-3.5 py-2 font-mono text-[12.5px] leading-normal whitespace-pre">
        <b className="font-semibold text-t-accent">a ▸ note on auth-fix → sol</b>
        {"  keep old tokens valid for a day"}
        <span className="inline-block w-[0.6em] bg-t-text"> </span>
      </div>
    </div>
  );
}
