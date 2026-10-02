import { Inline } from "../components/Inline";
import { LocalLink } from "../components/LocalLink";
import { Tag } from "../components/marks";
import { useStrings } from "../lang/useStrings";
import { HandoffScene } from "./HandoffScene";

export function Hero() {
  const { home } = useStrings();
  // What tmt runs with today, and what is on the way. Planned ones are labeled.
  const chips: { name: string; note?: string }[] = [
    { name: "Claude Code" },
    { name: "Codex" },
    { name: "tmux" },
    { name: "Herdr" },
    { name: home.yourHarness, note: home.planned },
    { name: home.yourServer, note: home.designing },
  ];
  return (
    <div className="pb-14">
      <div className="font-mono text-xs leading-none font-semibold tracking-[0.06em] text-accent before:text-dim before:content-['##_']">
        {home.eyebrow}
      </div>
      <h1 className="mt-2.5 mb-8 max-w-[22ch] font-mono text-[clamp(34px,6vw,64px)] leading-[1.0] font-bold tracking-[-0.02em] text-balance">
        <Inline text={home.title} />
      </h1>
      <div className="grid grid-cols-1 items-center gap-8 lg:grid-cols-[minmax(0,5fr)_minmax(0,7fr)] lg:gap-10">
        <div className="min-w-0">
          <p className="mb-3 font-mono text-[clamp(18px,2.2vw,22px)] leading-snug font-semibold text-balance">
            {home.tagline}
          </p>
          <p className="mb-4 text-[17px] leading-normal sm:text-[18px]">{home.lede}</p>
          <ul
            aria-label={home.worksWith}
            className="m-0 mb-6 flex list-none flex-wrap gap-1.5 p-0 font-mono text-xs"
          >
            {chips.map((chip) => (
              <li
                key={chip.name}
                className={`border-[1.5px] bg-sheet px-2.5 py-1 ${
                  chip.note ? "border-dashed border-dim text-muted" : "border-text text-text"
                }`}
              >
                {chip.name}
                {chip.note && <Tag kind="planned">{chip.note}</Tag>}
              </li>
            ))}
          </ul>
          <p className="m-0 font-mono text-sm">
            <LocalLink
              to="/"
              hash="install"
              className="inline-block border-2 border-text bg-accent px-3.5 py-2 font-semibold text-paper no-underline shadow-[3px_3px_0_var(--c-text)]"
            >
              {home.install}
            </LocalLink>
            <LocalLink to="/" hash="start" className="ml-4 text-accent">
              {home.start}
            </LocalLink>
          </p>
        </div>
        <HandoffScene />
      </div>
    </div>
  );
}
