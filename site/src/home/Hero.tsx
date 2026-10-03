import { Inline } from "../components/Inline";
import { LocalLink } from "../components/LocalLink";
import { Tag } from "../components/marks";
import { useStrings } from "../lang/useStrings";
import { HandoffScene } from "./HandoffScene";

export function Hero() {
  const { home } = useStrings();
  // Shipped and future integrations retain the existing explicit status labels.
  const chips: { name: string; note?: string }[] = [
    { name: "Claude Code" },
    { name: "Codex" },
    { name: "tmux" },
    { name: "Herdr" },
    { name: home.yourHarness, note: home.planned },
    { name: home.yourServer, note: home.designing },
  ];
  return (
    <div className="hero-band">
      <div className="grid grid-cols-1 items-center gap-8 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)] lg:gap-12">
        <div className="min-w-0">
          <div className="font-mono text-xs leading-none font-semibold tracking-[0.06em] text-accent before:text-dim before:content-['##_']">
            {home.eyebrow}
          </div>
          <h1 className="mt-4 mb-6 max-w-[22ch] font-mono text-[clamp(34px,4.2vw,62px)] leading-[1.04] font-bold tracking-[-0.035em] text-balance">
            <Inline text={home.title} />
          </h1>
          <p className="mb-6 max-w-[36ch] font-mono text-[clamp(17px,1.5vw,21px)] leading-snug font-semibold text-balance">
            {home.tagline}
          </p>
          <p className="m-0 flex flex-wrap items-center gap-4 font-mono text-sm">
            <LocalLink
              to="/"
              hash="install"
              className="inline-block border-2 border-text bg-accent px-4 py-2.5 font-semibold text-paper no-underline shadow-[3px_3px_0_var(--c-text)]"
            >
              {home.install}
            </LocalLink>
            <LocalLink to="/" hash="start" className="text-accent">
              {home.start}
            </LocalLink>
          </p>
        </div>
        <div className="min-w-0 bg-accent-soft p-3 sm:p-6">
          <HandoffScene />
        </div>
      </div>
      <div className="mt-10 border-t border-rule pt-6">
        <p className="mb-4 max-w-[80ch] text-[17px] leading-normal sm:text-[18px]">{home.lede}</p>
        <ul
          aria-label={home.worksWith}
          className="m-0 flex list-none flex-wrap gap-2 p-0 font-mono text-xs"
        >
          {chips.map((chip) => (
            <li
              key={chip.name}
              className={`border-[1.5px] bg-sheet px-2.5 py-1.5 ${
                chip.note ? "border-dashed border-dim text-muted" : "border-text text-text"
              }`}
            >
              {chip.name}
              {chip.note && <Tag kind="planned">{chip.note}</Tag>}
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
