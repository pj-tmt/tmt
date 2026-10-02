import { useStrings } from "../lang/useStrings";

// Each mark in the colour the board draws it in. Meanings are the canonical
// ones in design/cli-style.md; ▸ is drawn only on the board.
const TONE: Record<string, string> = {
  "●": "text-t-working",
  "○": "text-t-dim",
  "◌": "text-t-dim",
  "◆": "text-t-waiting",
  "✗": "text-t-blocked",
  "✓": "text-t-working",
  "↻": "text-t-muted",
  "▸": "text-t-accent",
};

export function MarkLegend() {
  const { marks } = useStrings().chapters;
  return (
    <div className="my-5">
      <p className="mb-3 text-[15px] leading-normal text-muted">{marks.intro}</p>
      <ul
        aria-label={marks.label}
        className="m-0 grid list-none grid-cols-2 gap-px overflow-hidden rounded-lg border border-term-edge bg-term-edge p-0 sm:grid-cols-4"
      >
        {marks.items.map((item) => (
          <li
            key={item.mark}
            className="grid min-w-0 content-start gap-1 bg-term p-3.5 text-t-text"
          >
            <span
              aria-hidden="true"
              className={`font-mono text-[40px] leading-none font-bold ${TONE[item.mark] ?? ""}`}
            >
              {item.mark}
            </span>
            <b className="font-mono text-[13px] font-semibold">
              {item.name}
              {item.mark === "▸" && (
                <span className="ml-1.5 font-normal text-t-dim">· {marks.boardOnly}</span>
              )}
            </b>
            <span className="text-[13px] leading-snug text-t-muted">{item.text}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
