import { Inline } from "../components/Inline";
import { LocalLink } from "../components/LocalLink";
import { Tag } from "../components/marks";
import { useStrings } from "../lang/useStrings";
import { BoardScene } from "./BoardScene";

export function Hero() {
  const { home } = useStrings();
  // What tmt runs with today, and what is on the way. Planned ones are labeled.
  const chips: { name: string; note?: string }[] = [
    { name: "Claude Code" },
    { name: "Codex" },
    { name: "tmux" },
    { name: home.yourHarness, note: home.planned },
    { name: home.yourServer, note: home.designing },
  ];
  return (
    <div>
      <div className="font-mono text-xs leading-none font-semibold tracking-[0.06em] text-accent before:text-dim before:content-['##_']">
        {home.eyebrow}
      </div>
      <h1 className="mt-2.5 mb-6 font-mono text-[clamp(32px,5.2vw,52px)] leading-[1.04] font-bold tracking-[-0.02em] text-balance">
        <Inline text={home.title} />
      </h1>
      <div className="grid grid-cols-1 items-center gap-8 md:grid-cols-[minmax(0,5fr)_minmax(0,6fr)]">
        <div className="min-w-0">
          <p className="mb-3.5 text-[17px] leading-normal sm:text-[18px]">{home.lede}</p>
          <p className="mt-5 mb-0 font-mono text-sm">
            <LocalLink
              to="/"
              hash="install"
              className="rounded-md bg-accent px-3.5 py-2.5 font-semibold text-paper no-underline"
            >
              {home.install}
            </LocalLink>
            <LocalLink to="/" hash="start" className="ml-4 text-accent">
              {home.start}
            </LocalLink>
          </p>
        </div>
        <BoardScene />
      </div>
      <ul
        aria-label={home.worksWith}
        className="mt-7 mb-0 flex list-none flex-wrap gap-1.5 p-0 font-mono text-xs"
      >
        {chips.map((chip) => (
          <li
            key={chip.name}
            className={`rounded-full border px-2.5 py-1 ${
              chip.note ? "border-dashed border-rule text-dim" : "border-rule text-muted"
            }`}
          >
            {chip.name}
            {chip.note && <Tag kind="planned">{chip.note}</Tag>}
          </li>
        ))}
      </ul>
    </div>
  );
}
