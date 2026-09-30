import type { ReactNode } from "react";

export type Status = "shipped" | "alpha" | "planned" | "designing" | "built in";

// The shipped/planned marker. Planned work is always labeled, so the handbook
// never presents a proposal as shipped behavior.
export function Tag({ kind, children }: { kind: Status; children?: ReactNode }) {
  const planned = kind === "planned" || kind === "designing";
  return (
    <span
      className={`ml-2 inline-block rounded-[3px] px-1.5 py-1 align-[0.2em] font-mono text-[10.5px] leading-none font-semibold tracking-[0.08em] whitespace-nowrap uppercase ${
        planned ? "bg-waiting-soft text-waiting" : "bg-accent-soft text-accent"
      }`}
    >
      {children ?? kind}
    </span>
  );
}

const STATE = {
  done: { mark: "●", tone: "text-accent" },
  now: { mark: "◐", tone: "text-text" },
  next: { mark: "○", tone: "text-muted" },
  wait: { mark: "◆", tone: "text-waiting" },
};

export function St({ kind, children }: { kind: keyof typeof STATE; children: ReactNode }) {
  const state = STATE[kind];
  return (
    <span className={`font-mono text-xs font-semibold whitespace-nowrap ${state.tone}`}>
      {state.mark} {children}
    </span>
  );
}

export function Callout({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="my-4 rounded-r-md border-l-[3px] border-waiting bg-waiting-soft px-3.5 py-3 text-[15.5px]">
      <b className="text-waiting">{title}</b> {children}
    </div>
  );
}

export function Aside({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="my-4 border-l-2 border-waiting py-0.5 pl-4 text-base text-muted">
      <strong className="text-text">{title}</strong> {children}
    </div>
  );
}

export function Caption({ children }: { children: ReactNode }) {
  return <p className="mt-2 mb-6 font-mono text-xs leading-normal text-muted">{children}</p>;
}

export function Lede({ children }: { children: ReactNode }) {
  return <p className="mb-3 text-[19px] leading-normal sm:text-[21px]">{children}</p>;
}

export function Waiting() {
  return <span className="text-waiting">◆</span>;
}

export function More({ summary, children }: { summary: string; children: ReactNode }) {
  return (
    <details className="my-4 rounded-md border border-rule bg-sheet px-3 py-2">
      <summary className="cursor-pointer font-mono text-sm text-muted">{summary}</summary>
      <div className="mt-2">{children}</div>
    </details>
  );
}

export function Terms({ children }: { children: ReactNode }) {
  return (
    <dl className="my-5 grid grid-cols-1 gap-1 sm:grid-cols-[max-content_minmax(0,1fr)] sm:gap-x-7 sm:gap-y-3.5 [&_dd]:m-0 [&_dd]:mb-3 sm:[&_dd]:mb-0 [&_dt]:font-display [&_dt]:text-sm [&_dt]:leading-relaxed [&_dt]:font-semibold">
      {children}
    </dl>
  );
}

export function KeyList({ children }: { children: ReactNode }) {
  return (
    <ul className="my-4 list-none p-0 [&>li]:my-1.5 [&>li]:border-l-[3px] [&>li]:border-accent [&>li]:py-1.5 [&>li]:pl-3.5">
      {children}
    </ul>
  );
}

export function Soon({ items }: { items: [string, string][] }) {
  return (
    <div className="my-2.5 flex flex-wrap gap-x-5 gap-y-2.5 font-mono text-sm text-muted">
      {items.map(([name, kind]) => (
        <span key={name}>
          <b className="font-semibold text-text">{name}</b> {kind}
        </span>
      ))}
    </div>
  );
}
