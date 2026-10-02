import { Link } from "@tanstack/react-router";
import { Tag } from "../components/marks";
import { BoardScene } from "./BoardScene";

// What tmt runs with today, and what is on the way. Planned ones are labeled.
const chips: { name: string; note?: "planned" | "designing" }[] = [
  { name: "Claude Code" },
  { name: "Codex" },
  { name: "tmux" },
  { name: "your harness", note: "planned" },
  { name: "your server", note: "designing" },
];

export function Hero() {
  return (
    <div>
      <div className="font-mono text-xs leading-none font-semibold tracking-[0.06em] text-accent before:text-dim before:content-['##_']">
        home
      </div>
      <h1 className="mt-2.5 mb-6 font-mono text-[clamp(32px,5.2vw,52px)] leading-[1.04] font-bold tracking-[-0.02em] text-balance">
        {"One simple core. "}
        <em className="text-accent not-italic">Endless</em>
        {" ways for AI to work together."}
      </h1>
      <div className="grid grid-cols-1 items-center gap-8 md:grid-cols-[minmax(0,5fr)_minmax(0,6fr)]">
        <div className="min-w-0">
          <p className="mb-3.5 text-[17px] leading-normal sm:text-[18px]">
            tmt passes messages between agents and keeps a receipt for every reply. The board, colab
            and meet are built on that one core. It is made for any agent, any harness and any
            machine; the list below shows what runs today and what is on the way.
          </p>
          <ul className="m-0 flex list-none flex-wrap gap-1.5 p-0 font-mono text-xs">
            {chips.map((chip) => (
              <li
                key={chip.name}
                className={`rounded-full border px-2.5 py-1 ${
                  chip.note ? "border-dashed border-rule text-dim" : "border-rule text-muted"
                }`}
              >
                {chip.name}
                {chip.note && <Tag kind={chip.note} />}
              </li>
            ))}
          </ul>
          <p className="mt-5 mb-0 font-mono text-sm">
            <Link
              to="/"
              hash="install"
              className="rounded-md bg-accent px-3.5 py-2.5 font-semibold text-paper no-underline"
            >
              Install
            </Link>
            <Link to="/" hash="start" className="ml-4 text-accent">
              Start with one message ↓
            </Link>
          </p>
        </div>
        <BoardScene />
      </div>
    </div>
  );
}
