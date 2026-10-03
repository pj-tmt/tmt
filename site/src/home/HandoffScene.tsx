import { useStrings } from "../lang/useStrings";
import { Pane, cmd, dim, ok, warn, type Row } from "../scenes/Pane";
import { useFrames } from "../scenes/useFrames";
import { Window } from "../scenes/Window";

// One request between two different providers, in four frames: Claude Code
// asks Codex, Codex replies with the receipt it was given, and the reply lands
// back. Every line is one the working chapter documents. The last frame is
// the one shown under reduced motion.
const FRAMES = 4;
const REST = 3;

const reviewerRows: Row[] = [
  { from: 0, node: cmd('tmt talk builder "Review the diff"') },
  // talk prints nothing while it waits, then the completion line and the reply as plain text.
  { from: 3, node: <>{ok("✓")} Completed request req_9ba4… for builder (%7)</> },
  { from: 3, node: "No blocking issues" },
];
const builderRows: Row[] = [
  { from: 1, node: warn('<tmt-reply from="reviewer">') },
  { from: 1, node: "Review the diff" },
  { from: 2, node: dim("…reviewing…") },
  { from: 2, node: cmd('tmt reply req_9ba4… --receipt v2_… --message "No blocking issues"') },
  { from: 3, node: ok("✓ Submitted response for request req_9ba4…") },
];

export function HandoffScene() {
  const { handoff } = useStrings().home;
  const { ref, index } = useFrames<HTMLDivElement>(FRAMES, { intervalMs: 2300, rest: REST });
  // The ids that tie the two sides appear one by one and stay.
  const tie = [
    { show: index >= 0, text: `${handoff.request} req_9ba4…` },
    { show: index >= 1, text: `${handoff.receipt} v2_…` },
    { show: index >= 2, text: handoff.reply },
  ];
  return (
    <div ref={ref}>
      <Window
        title={handoff.title}
        label={handoff.label}
        footer={
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 bg-term-bar px-3 py-2 font-mono text-[11.5px] text-t-muted">
            {tie.map((item, k) => (
              <span
                key={item.text}
                className={`transition-opacity duration-300 motion-reduce:transition-none ${
                  item.show ? "opacity-100" : "opacity-0"
                }`}
              >
                {k > 0 && <span className="mr-2 text-t-dim">⇄</span>}
                {item.text}
              </span>
            ))}
            <span
              className={`ml-auto font-semibold transition-opacity duration-300 motion-reduce:transition-none ${
                index >= 3 ? "text-t-working opacity-100" : "opacity-0"
              }`}
            >
              ✓ {handoff.tied}
            </span>
          </div>
        }
      >
        <div aria-hidden="true" className="grid grid-cols-1 gap-px bg-term-edge">
          <Pane
            name={handoff.reviewer}
            tag="Claude Code"
            tone="text-t-review"
            rows={reviewerRows}
            step={index}
          />
          <Pane
            name={handoff.builder}
            tag="Codex"
            tone="text-t-link"
            rows={builderRows}
            step={index}
          />
        </div>
      </Window>
    </div>
  );
}
