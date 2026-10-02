import { Inline } from "../components/Inline";
import { useStrings } from "../lang/useStrings";

// The three ideas the rest of the handbook builds on, each next to the command
// that makes it. The receipt ties a result to its request.
export function Concepts() {
  const { concepts } = useStrings().home;
  return (
    <div className="my-6">
      <ol className="m-0 grid list-none grid-cols-1 gap-3 p-0 md:grid-cols-3">
        {concepts.items.map((item, k) => (
          <li key={item.name} className="min-w-0 rounded-lg border border-rule bg-sheet p-3.5">
            <div className="font-mono text-xs text-dim">{k + 1}</div>
            <b className="block font-mono text-lg text-text">{item.name}</b>
            <p className="mt-1.5 mb-0 text-[15px] leading-snug text-muted">
              <Inline text={item.text} />
            </p>
          </li>
        ))}
      </ol>
      <p className="mt-3 mb-0 flex items-baseline gap-2 font-mono text-[13px] text-text">
        <span className="rounded-[3px] bg-accent-soft px-1.5 py-0.5 text-accent">
          {concepts.receipt}
        </span>
        <span className="font-body text-[15px] text-muted">
          <Inline text={concepts.receiptText} />
        </span>
      </p>
    </div>
  );
}
