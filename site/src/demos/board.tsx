import type { ReactNode } from "react";
import { dm, line, pad, type Line } from "./segments";

// One board frame in the squad walkthroughs.
export type Row = [name: string, state: string, task: string, pr: string, waiting?: boolean];
export type BoardSpec = {
  rows: Row[];
  sel?: number;
  detail?: string;
  pending?: string;
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
      {line(dm(`lead sol · ${board.rows.length} members`))}
    </>,
  );
  push("");
  push(line(dm("/ search")));
  if (board.rows.length) {
    push(line(dm(`    ${pad("MEMBER", 13)}${pad("STATE", 10)}${pad("TASK", 28)}PR`)));
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
          {selected ? "▸ " : "  "}
          {waiting ? <span className="text-t-waiting">◆ </span> : "  "}
          {pad(name, 13)}
          <span className={tone}>{pad(state, 10)}</span>
          {pad(task, 28)}
          {pr || "–"}
        </>
      );
      push(selected ? <span className="bg-t-selection">{content} </span> : content);
    });
  } else {
    push(line(dm("  no members yet")));
  }
  push("");
  if (board.detail) push(line(dm(board.detail)));
  if (board.pending)
    push(
      <>
        <span className="text-t-waiting">waiting on you</span>
        {"  "}
        {board.pending}
      </>,
    );
  if (board.to) {
    push(
      <>
        <span className="text-t-accent">t ▸ {board.to}</span>
        {"  "}
        {typed}
        {cursor && <span className="cursor"> </span>}
      </>,
    );
    (board.reply ?? []).slice(0, replies).forEach((reply) => push(line(reply)));
  }
  push("");
  push(line(dm("⏎ jump  ⌫ back  / search  t talk  r reply  a annotate  o open  y copy")));
  return lines;
}

const keys: [string, string][] = [
  ["⏎", "jump"],
  ["⌫", "back"],
  ["/", "search"],
  ["t", "talk"],
  ["r", "reply"],
  ["a", "annotate"],
  ["o", "open"],
  ["y", "copy"],
  ["tab", "pane"],
  ["d", "toggle detail"],
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
      aria-label="Sketch of the default team board: rows for auth-fix, docs-sweep, perf-cache and old-spike, with auth-fix selected, marked as waiting on you and its decision on a second line, and old-spike dimmed with its age; details for auth-fix and the latest reply to its right; the lead's notes below; and a key legend."
      className={sketch}
    >
      <pre className={`${pre} border-b border-term-edge`}>
        <span className="bg-t-selection font-bold text-t-waiting">{" product ◆1 ✗1 "}</span>
        <span className="text-t-muted">{" reviews  infra "}</span>
        {"\n"}
        <span className="text-t-muted">lead sol · 4 members</span>
      </pre>
      <div className="grid grid-cols-1 sm:grid-cols-[minmax(0,62fr)_minmax(0,38fr)]">
        <pre className={`${pre} border-b border-term-edge sm:border-r sm:border-b-0`}>
          <span className="text-t-muted">rows</span>
          {"\n"}
          <span className="text-t-muted">{"    MEMBER      STATE     TASK            PR"}</span>
          {"\n"}
          <span className="bg-t-selection">
            {"  "}
            <span className="text-t-waiting">◆</span>
            {" auth-fix    "}
            <span className="text-t-blocked">blocked</span>
            {"   rotate session… #412 draft"}
            {"\n"}
            {"                          "}
            {"approve rotation plan     "}
          </span>
          {"\n    docs-sweep  "}
          <span className="text-t-review">review</span>
          {"    install guide   #409 open\n    perf-cache  "}
          <span className="text-t-working">working</span>
          {"   cache reads     "}
          <span className="text-t-dim">–</span>
          {"\n"}
          <span className="text-t-dim">
            {"    old-spike   idle      parser spike    – stale 3h"}
          </span>
        </pre>
        <div>
          <pre className={`${pre} border-b border-term-edge`}>
            <span className="text-t-muted">detail</span>
            {"\n"}
            <span className="font-bold">auth-fix</span>
            {"\n"}
            <span className="text-t-waiting">waiting on you:</span>
            {" approve\n  rotation plan\ntask: rotate session tokens\npr: #412 draft"}
          </pre>
          <pre className={pre}>
            <span className="text-t-muted">replies</span>
            {"\nsol · 2m  noted, passing it on"}
          </pre>
        </div>
      </div>
      <pre className={`${pre} border-t border-term-edge`}>
        <span className="text-t-muted">notes · sol</span>
        {"\n"}
        <span className="font-bold text-t-accent">## Now</span>
        {
          "\n- tokens: waiting on Ben's call (login vs sweep)\n- install guide: one page, platform tabs"
        }
      </pre>
      <KeyBar />
    </div>
  );
}

export function SplitBoardSketch() {
  return (
    <div
      role="img"
      aria-label="Sketch of a split board: rows on the left with auth-fix selected, showing the lead's one-line note and an annotation sent to sol; the lead's notes on the right; and an annotation being typed."
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
          <span className="text-t-dim">note</span>
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
