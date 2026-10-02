import type { ReactNode } from "react";

// A pane of a terminal scene: lines that show from the step that writes them.
// Shared by the chapter scenes and the home hero.
export type Row = { from: number; node: ReactNode };

export const cmd = (text: string) => (
  <>
    <span className="text-t-accent">$ </span>
    {text}
  </>
);
export const dim = (text: string) => <span className="text-t-dim">{text}</span>;
export const ok = (text: string) => <span className="text-t-working">{text}</span>;
export const warn = (text: string) => <span className="text-t-waiting">{text}</span>;

export function Pane({
  name,
  tag,
  tone,
  rows,
  step,
}: {
  name: string;
  tag: string;
  tone: string;
  rows: Row[];
  step: number;
}) {
  return (
    <div className="min-w-0 bg-term p-3 font-mono text-[12.5px] leading-[1.55] text-t-text">
      <div className="mb-2 flex justify-between text-t-muted">
        <b className="text-t-text">{name}</b>
        <span className={tone}>{tag}</span>
      </div>
      <div className="min-h-[7.5em]">
        {rows.map((row, index) => (
          <div
            key={index}
            className={`break-words transition-opacity duration-300 motion-reduce:transition-none ${
              row.from <= step ? "opacity-100" : "opacity-0"
            }`}
          >
            {row.node}
          </div>
        ))}
      </div>
    </div>
  );
}
