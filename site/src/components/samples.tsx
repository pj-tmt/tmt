import type { ReactNode } from "react";
import tokens from "../../../design/tokens/tokens.json";

const termBlock = "my-5 w-full overflow-hidden rounded-md bg-term text-t-text";

export function LsSample() {
  const dm = "text-t-dim";
  return (
    <pre
      className={`${termBlock} term-scroll px-4 py-3.5 font-mono text-[13px] leading-[1.65] sm:text-sm`}
    >
      <b>SAVED</b> <span className={dm}>3</span>
      {"\n  "}
      <span className="text-t-accent">●</span>
      {"  reviewer   "}
      <span className="text-t-review">claude</span>
      <span className={dm}>:</span>
      {"7c41e9d2  ~/web\n  "}
      <span className="text-t-accent">●</span>
      {"  builder    "}
      <span className="text-t-link">codex</span>
      <span className={dm}>:</span>
      {"019a2f4c   ~/web\n  "}
      <span className={dm}>○</span>
      {"  planner    "}
      <span className="text-t-review">claude</span>
      <span className={dm}>:</span>
      {"3f9a1c07             "}
      <span className="text-t-accent">↻ tmt resume planner</span>
      {"\n    "}
      <span className={dm}>offline: gemini</span>
      {"\n\n"}
      <b>TEMPORARY</b> <span className={dm}>2</span>
      {"\n  "}
      <span className="text-t-accent">●</span>
      {"  server-log "}
      <span className={dm}>tmux:</span>
      {"%31     ~/web\n  "}
      <span className={dm}>◌</span>
      {"  scratch    "}
      <span className={dm}>tmux:</span>
      {"%34     ~/web      "}
      <span className="text-t-accent">shell</span>
    </pre>
  );
}

export function Badge({ name, state }: { name: string; state: "running" | "ended" | "unknown" }) {
  return (
    <span
      className={`inline-flex items-center gap-[0.45em] font-mono text-[13px] whitespace-nowrap ${state === "ended" ? "opacity-50" : ""}`}
    >
      {state === "running" && <span className="text-accent">●</span>}
      {state === "ended" && "○"}
      {name} (tmt)
    </span>
  );
}

export function States({ items }: { items: [ReactNode, string, ReactNode][] }) {
  return (
    <div className="my-4 border-t border-rule">
      {items.map(([badge, title, text], index) => (
        <div
          key={index}
          className="grid grid-cols-1 gap-1 border-b border-rule py-3.5 sm:grid-cols-[170px_minmax(0,1fr)] sm:gap-4.5"
        >
          <span>{badge}</span>
          <span>
            <strong>{title}</strong> {text}
          </span>
        </div>
      ))}
    </div>
  );
}

export function DriverCard({
  name,
  kind,
  spec,
}: {
  name: string;
  kind: string;
  spec: [string, ReactNode][];
}) {
  return (
    <article className="my-6 overflow-hidden rounded-md border border-rule bg-sheet">
      <header className="flex flex-wrap items-baseline gap-x-3.5 gap-y-1.5 border-b border-rule px-4.5 py-3.5">
        <h3 className="m-0 font-display text-xl font-extrabold">{name}</h3>
        <span className="font-mono text-xs text-muted">{kind}</span>
        <span className="ml-auto rounded-[3px] bg-accent-soft px-1.5 py-1 font-mono text-[10.5px] leading-none font-semibold tracking-[0.08em] text-accent uppercase">
          built in
        </span>
      </header>
      <dl className="m-0 grid grid-cols-1 gap-0.5 px-4.5 py-4 text-[16.5px] sm:grid-cols-[120px_minmax(0,1fr)] sm:gap-x-4.5 sm:gap-y-2.5">
        {spec.map(([term, value]) => (
          <div key={term} className="contents">
            <dt className="font-mono text-[11px] leading-[1.9] font-semibold tracking-[0.06em] text-muted uppercase">
              {term}
            </dt>
            <dd className="m-0 mb-2.5 sm:mb-0">{value}</dd>
          </div>
        ))}
      </dl>
    </article>
  );
}

// Three pane borders showing the badge in each state.
export function BadgeBorders() {
  const panes: [string, "running" | "ended" | "unknown", string, string, string][] = [
    ["1", "running", "reviewer", "tmt/main", "running: the dot is green"],
    ["2", "ended", "builder", "tmt/fix", "ended: the whole badge dims"],
    ["3", "unknown", "sol", "docs", "unknown: no dot, no styling"],
  ];
  return (
    <div
      role="img"
      aria-label="Three tmux panes. Their top borders show the pane number, then a badge: a green dot with reviewer (tmt) for a running session, a dimmed hollow dot with builder (tmt) for an ended one, and sol (tmt) with no dot when the state is unknown."
      className={`${termBlock} grid grid-cols-1 sm:grid-cols-3`}
    >
      {panes.map(([index, state, name, branch, note]) => (
        <div
          key={index}
          className="min-w-0 border-b border-term-edge last:border-0 sm:border-r sm:border-b-0"
        >
          <div className="flex items-center gap-0.5 overflow-hidden px-2 pt-2 font-mono text-[11.5px] leading-none whitespace-nowrap text-t-dim">
            <span>─ {index} </span>
            <b className={`font-normal text-t-text ${state === "ended" ? "opacity-45" : ""}`}>
              [{state === "running" && <i className="text-t-working not-italic">● </i>}
              {state === "ended" && "○ "}
              {name} (tmt)]
            </b>
            <span className="mx-1 min-w-2 flex-1 border-t border-t-dim/50" />
            <span>{branch} ─</span>
          </div>
          <pre className="m-0 px-2.5 pt-3 pb-4.5 font-mono text-xs whitespace-pre-wrap text-t-dim">
            {note}
          </pre>
        </div>
      ))}
    </div>
  );
}

export function Spaces() {
  return (
    <div
      role="img"
      aria-label="An example layout: leads sol and rin in one place; members auth-fix, docs-sweep and perf-cache in another, each with its worktree and branch, and auth-fix marked as waiting on you."
      className="my-5 grid grid-cols-1 gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,2fr)]"
    >
      <Space
        title="leads"
        rows={[
          ["●", "sol", "product"],
          ["●", "rin", "reviews"],
        ]}
      />
      <Space
        title="members"
        rows={[
          ["◆", "auth-fix", "~/w/app-3 · fix/token"],
          ["●", "docs-sweep", "~/w/app-1 · docs/install"],
          ["●", "perf-cache", "~/w/app-2 · perf/rooms"],
        ]}
      />
    </div>
  );
}

function Space({ title, rows }: { title: string; rows: [string, string, string][] }) {
  return (
    <div className="rounded-lg border border-rule bg-sheet px-3.5 py-3">
      <h4 className="m-0 mb-2 font-mono text-[10.5px] leading-none font-semibold tracking-[0.08em] text-muted uppercase">
        {title}
      </h4>
      <ul className="m-0 list-none p-0 font-mono text-[13px] leading-[1.9]">
        {rows.map(([mark, name, where]) => (
          <li key={name} className="truncate">
            <span className={mark === "◆" ? "text-waiting" : "text-accent"}>{mark}</span>{" "}
            <b className="font-semibold">{name}</b> <span className="text-muted">{where}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

export function ThreadsMock() {
  const item = "px-3.5 py-2 font-mono text-[13px] leading-none";
  const head =
    "px-3.5 pt-3 pb-1.5 font-mono text-[10.5px] leading-none font-semibold tracking-[0.08em] text-muted uppercase";
  return (
    <div
      role="img"
      aria-label="Sketch of the threads app: a sidebar of agents with running and ended dots, and an open thread where builder asks reviewer for a review and reviewer replies."
      className="my-6 grid grid-cols-1 overflow-hidden rounded-lg border border-rule bg-sheet text-[15px] sm:grid-cols-[170px_minmax(0,1fr)]"
    >
      <div className="hidden border-r border-rule bg-accent-soft py-3.5 sm:block">
        <div className={head}>agents</div>
        <div className={`${item} bg-sheet font-semibold`}>
          <span className="text-accent">●</span> reviewer
        </div>
        <div className={item}>
          <span className="text-accent">●</span> builder
        </div>
        <div className={`${item} opacity-50`}>○ sol</div>
        <div className={head}>waiting</div>
        <div className={item}>
          <span className="mr-1 inline-block min-w-[18px] rounded-full bg-waiting px-1.5 py-0.5 text-center text-[11px] text-sheet">
            2
          </span>{" "}
          incoming
        </div>
      </div>
      <div className="flex min-w-0 flex-col gap-3 px-4.5 py-3.5">
        <div className="border-b border-rule pb-2.5 font-display text-sm font-semibold">
          reviewer <span className="ml-2 text-xs font-normal text-muted">● running · pane 0.2</span>
        </div>
        <Message who="builder" at="10:24" text="Review the diff and list concrete risks." />
        <Message
          who="reviewer"
          at="10:31 · reply"
          reply
          text="Two risks: the retry loop can resend after a timeout, and the new flag is never read."
        />
        <Message who="you" at="10:33" text="Fix the first one and send me the patch." />
        <div className="rounded-md border border-rule px-2.5 py-2 text-sm text-muted">
          Reply to reviewer…
        </div>
      </div>
    </div>
  );
}

function Message({
  who,
  at,
  text,
  reply,
}: {
  who: string;
  at: string;
  text: string;
  reply?: boolean;
}) {
  return (
    <div className={reply ? "border-l-2 border-accent pl-2.5" : ""}>
      <b className="font-display text-[13px] font-semibold">{who}</b>
      <span className="ml-2 font-mono text-[11px] text-muted">{at}</span>
      <p className="mt-1 mb-0">{text}</p>
    </div>
  );
}

type Rendering = { use: string; dark: string; light: string; terminal?: string };

function Swatch({ color }: { color: string }) {
  return (
    <span
      className="mr-1 inline-block size-[0.9em] rounded-[2px] border border-rule align-[-0.1em]"
      style={{ background: color }}
    />
  );
}

// The color tokens, read from tokens.json: one table for the handbook and the
// design page, so neither copies the values.
export function TokenTable({ group = "color" }: { group?: "color" | "surface" }) {
  const rows = Object.entries(tokens[group] as Record<string, Rendering>);
  return (
    <div className="term-scroll my-4 w-full">
      <table>
        <thead>
          <tr>
            <th>Token</th>
            <th>Used for</th>
            <th>Dark</th>
            <th>Light</th>
            {group === "color" && <th>Terminal</th>}
          </tr>
        </thead>
        <tbody>
          {rows.map(([name, value]) => (
            <tr key={name}>
              <td>{name}</td>
              <td>{value.use}</td>
              <td className="whitespace-nowrap">
                <Swatch color={value.dark} />
                <code>{value.dark}</code>
              </td>
              <td className="whitespace-nowrap">
                <Swatch color={value.light} />
                <code>{value.light}</code>
              </td>
              {group === "color" && <td className="whitespace-nowrap">{value.terminal}</td>}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// Each color token in the three places it appears: a light page, a dark page
// and a terminal running the default theme.
export function TokenPreview() {
  const colors = Object.entries(tokens.color as Record<string, Rendering>).filter(
    ([name]) => name !== "selection",
  );
  const panel = (
    title: string,
    background: string,
    pick: (value: Rendering) => string,
    note: string,
  ) => (
    <div className="min-w-0 overflow-hidden rounded-md border border-rule" style={{ background }}>
      <div
        className="px-3 pt-2.5 pb-1 font-mono text-[10.5px] font-semibold tracking-[0.08em] uppercase"
        style={{ color: pick(tokens.color.muted) }}
      >
        {title}
      </div>
      <ul className="m-0 list-none px-3 pb-3 font-mono text-[13px] leading-[1.9]">
        {colors.map(([name, value]) => (
          <li key={name} style={{ color: pick(value) }}>
            ● {name}
          </li>
        ))}
      </ul>
      <div
        className="border-t px-3 py-1.5 font-mono text-[11px]"
        style={{ color: pick(tokens.color.dim), borderColor: pick(tokens.surface.rule) }}
      >
        {note}
      </div>
    </div>
  );
  return (
    <div className="my-5 grid grid-cols-1 gap-3 sm:grid-cols-3">
      {panel(
        "light page",
        tokens.surface.paper.light,
        (value) => value.light,
        "prefers-color-scheme: light",
      )}
      {panel(
        "dark page",
        tokens.surface.paper.dark,
        (value) => value.dark,
        "prefers-color-scheme: dark",
      )}
      {panel(
        "terminal · tmt theme",
        tokens.surface.term.dark,
        (value) => value.dark,
        "ANSI 16 with the terminal theme",
      )}
    </div>
  );
}

export function TypePreview() {
  const fonts = Object.entries(tokens.font);
  const sample: Record<string, ReactNode> = {
    display: (
      <span className="font-display text-3xl font-extrabold tracking-[-0.015em]">
        One channel for all your agents
      </span>
    ),
    body: <span className="font-body text-lg">tmt keeps the reply, even across restarts.</span>,
    mono: <span className="font-mono text-sm">$ tmt talk reviewer "Review the diff."</span>,
  };
  return (
    <div className="my-5 border-t border-rule">
      {fonts.map(([name, value]) => (
        <div
          key={name}
          className="grid grid-cols-1 gap-1 border-b border-rule py-3.5 sm:grid-cols-[140px_minmax(0,1fr)] sm:gap-4"
        >
          <span className="font-mono text-xs text-muted">
            {name}
            <br />
            {value.use}
          </span>
          <span className="min-w-0 break-words">{sample[name]}</span>
        </div>
      ))}
    </div>
  );
}

export function MarkList() {
  return (
    <div className="my-5 grid grid-cols-1 gap-x-6 sm:grid-cols-2">
      {Object.entries(tokens.mark).map(([mark, meaning]) => (
        <div key={mark} className="flex gap-3 border-b border-rule py-2">
          <span className="w-8 shrink-0 whitespace-nowrap font-mono text-accent">{mark}</span>
          <span>{meaning}</span>
        </div>
      ))}
    </div>
  );
}
