import { useStrings } from "../lang/useStrings";
import { Pane, cmd, dim, ok, warn, type Row } from "../scenes/Pane";
import { useFrames } from "../scenes/useFrames";

// One request, step by step: lead talks, tmt stores and delivers, builder
// replies with the receipt, tmt stores the reply. A line shows from the step
// that writes it on. The last step is the complete picture, and the one shown
// under reduced motion.
const STEPS = 4;
const REST = 3;

const leadRows: Row[] = [
  { from: 0, node: cmd('tmt talk builder "rotate tokens"') },
  // talk prints nothing while it waits, then the completion line and the reply as plain text.
  { from: 3, node: <>{ok("✓")} Completed request req_9ba4… for builder (%7)</> },
  { from: 3, node: "Done" },
];
const tmtRows: Row[] = [
  { from: 0, node: <>stored request {warn("req_9ba4…")}</> },
  { from: 1, node: "→ typed into builder's pane" },
  { from: 3, node: "← reply stored with its receipt" },
  { from: 3, node: "→ handed back to lead" },
];
const builderRows: Row[] = [
  { from: 2, node: <>{warn('<tmt-reply from="lead">')}</> },
  { from: 2, node: "rotate tokens" },
  { from: 2, node: dim("…working…") },
  { from: 2, node: cmd('tmt reply req_9ba4… --receipt v2_… --message "Done"') },
  { from: 3, node: ok("✓ Submitted response for request req_9ba4…") },
];

// Where the request sits over the three panes at steps 0 to 2; the reply step hides it.
const PACKET = ["16%", "50%", "84%"];

export function MessageTravel() {
  const { chapters } = useStrings();
  const { travel } = chapters;
  const { ref, index } = useFrames<HTMLDivElement>(STEPS, { intervalMs: 1900, rest: REST });
  return (
    <div ref={ref} className="my-5">
      <div
        role="img"
        aria-label={travel.label}
        className="relative overflow-hidden rounded-lg border border-term-edge"
      >
        <div aria-hidden="true" className="grid grid-cols-1 gap-px bg-term-edge md:grid-cols-3">
          <Pane name={travel.lead} tag="claude" tone="text-t-review" rows={leadRows} step={index} />
          <Pane
            name={travel.tmtPane}
            tag={travel.exchange}
            tone="text-t-muted"
            rows={tmtRows}
            step={index}
          />
          <Pane
            name={travel.builder}
            tag="codex"
            tone="text-t-link"
            rows={builderRows}
            step={index}
          />
        </div>
        <span
          aria-hidden="true"
          style={{ left: PACKET[Math.min(index, 2)] }}
          className={`pointer-events-none absolute top-[58%] hidden -translate-x-1/2 rounded bg-t-waiting px-2 py-0.5 font-mono text-[11px] font-semibold text-term-bar transition-[left,opacity] duration-700 ease-in-out motion-reduce:transition-none md:block ${
            index < REST ? "opacity-100" : "opacity-0"
          }`}
        >
          req_9ba4
        </span>
      </div>
      <ol className="m-0 mt-3.5 grid list-none grid-cols-1 gap-3 p-0 md:grid-cols-3">
        {travel.steps.map((step, k) => {
          const on = k === 0 ? index === 0 : k === 1 ? index === 1 || index === 2 : index === 3;
          return (
            <li
              key={step.title}
              className={`border-t-2 pt-2 text-[13.5px] leading-snug transition-colors motion-reduce:transition-none ${
                on ? "border-waiting text-text" : "border-rule text-muted"
              }`}
            >
              <b className="block font-mono text-accent">{step.title}</b>
              {step.text}
            </li>
          );
        })}
      </ol>
      <p className="mt-3 mb-0 font-mono text-xs leading-normal text-muted">{travel.examples}</p>
    </div>
  );
}
