import { useState } from "react";
import { useStrings } from "../lang/useStrings";
import { useFrames } from "../scenes/useFrames";
import { Window } from "../scenes/Window";

// A sketch of the shared page. Comments arrive one by one and the docs item
// ticks off when the last arrives; the decision card is the one thing that
// waits on you and can be answered here. Under reduced motion the page rests
// with every comment in.
const COMMENTS = 4;
const WHO = [
  { name: "builder", machine: 0, tone: "text-t-link", edge: "border-l-(--t-link)" },
  { name: "tester", machine: 1, tone: "text-t-link", edge: "border-l-(--t-link)" },
  { name: "Mei", machine: 2, tone: "text-t-working", edge: "border-l-(--t-working)" },
  { name: "docs", machine: 2, tone: "text-t-review", edge: "border-l-(--t-review)" },
];

export function ColabPage() {
  const { colab } = useStrings().chapters;
  const [decision, setDecision] = useState<string | null>(null);
  const { ref, index } = useFrames<HTMLDivElement>(COMMENTS + 1, {
    intervalMs: 2200,
    rest: COMMENTS,
  });
  const shown = index;
  const done = [shown >= 1, shown >= 2, shown >= 4, false];
  // Who finished each plan item, in plan order.
  const owners = ["builder", "tester", "docs"];
  return (
    <div ref={ref} role="group" aria-label={colab.label} className="my-5">
      <Window
        title={colab.pageTitle}
        footer={
          <div className="bg-term-bar px-3 py-1.5 font-mono text-[11px] text-t-dim">
            {colab.sketch}
          </div>
        }
      >
        <div className="grid grid-cols-1 gap-4 p-4 md:grid-cols-2">
          <div className="min-w-0">
            <h5 className="m-0 mb-2 font-body text-base font-semibold">{colab.planHeading}</h5>
            <ol className="m-0 mb-3 grid list-none gap-1 p-0 font-body text-sm">
              {colab.items.map((item, k) => (
                <li key={item}>
                  {k + 1}. {item}
                  {done[k] ? (
                    <span className="ml-1.5 font-mono text-xs text-t-working">✓ {owners[k]}</span>
                  ) : k === 3 ? (
                    <span className="ml-1.5 font-mono text-xs text-t-waiting">
                      {decision ? `✓ ${colab.you}: ${decision}` : "◆"}
                    </span>
                  ) : (
                    <span className="ml-1.5 font-mono text-xs text-t-muted">{colab.working}</span>
                  )}
                </li>
              ))}
            </ol>
            <div className="rounded-md border border-(--t-waiting) bg-t-waiting/10 p-2.5 text-[13px] leading-snug">
              {decision ? (
                <>
                  <b className="font-mono text-t-working">✓ {colab.decided}</b> {decision} ·{" "}
                  {colab.everyone}
                </>
              ) : (
                <>
                  <b className="font-mono text-t-waiting">◆ {colab.needsYou}</b> {colab.question}
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    {[colab.shipTonight, colab.waitForDocs].map((choice) => (
                      <button
                        key={choice}
                        type="button"
                        onClick={() => setDecision(choice)}
                        className="cursor-pointer rounded border border-term-edge bg-term-bar px-2.5 py-1.5 font-mono text-xs font-semibold text-t-text hover:border-(--t-accent)"
                      >
                        {choice}
                      </button>
                    ))}
                  </div>
                </>
              )}
            </div>
          </div>
          <div className="grid min-w-0 content-start gap-2">
            {colab.comments.slice(0, shown).map((text, k) => (
              <div
                key={text}
                className={`animate-[grow_0.35s_ease-out] border-l-[3px] py-1 pl-2.5 text-[13px] leading-snug motion-reduce:animate-none ${WHO[k].edge}`}
              >
                <b className={`block font-mono text-[11px] ${WHO[k].tone}`}>
                  {WHO[k].name}
                  <em className="ml-1.5 font-normal text-t-dim not-italic">
                    {colab.machines[WHO[k].machine]}
                  </em>
                </b>
                {text}
              </div>
            ))}
            {decision && (
              <div className="border-l-[3px] border-l-(--t-waiting) py-1 pl-2.5 text-[13px]">
                <b className="block font-mono text-[11px] text-t-waiting">
                  {colab.you}
                  <em className="ml-1.5 font-normal text-t-dim not-italic">{colab.decision}</em>
                </b>
                {decision}.
              </div>
            )}
          </div>
        </div>
      </Window>
      <ol className="m-0 mt-3.5 grid list-none grid-cols-1 gap-3 p-0 md:grid-cols-3">
        {colab.roadmap.map((step, k) => (
          <li
            key={step.title}
            className={`border-t-[3px] pt-2 text-[13.5px] leading-snug ${
              k === 0 ? "border-waiting text-text" : "border-rule text-muted"
            }`}
          >
            <b className="block font-mono text-text">{step.title}</b>
            {step.text}
          </li>
        ))}
      </ol>
    </div>
  );
}
