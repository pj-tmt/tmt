import { useState, type ReactNode } from "react";
import { Inline } from "../components/Inline";
import { Tag } from "../components/marks";
import { useStrings } from "../lang/useStrings";
import { useFrames } from "../scenes/useFrames";
import { BoardStepScene, ColabScene, MeetScene, TalkScene } from "./journey-scenes";

// Each step adds one layer on the same foundation: stopping at any step leaves
// something that works. Only the first two are released; the others say so.
// The words (title, hint, status label, caption) come from the language's strings.
const steps: { id: string; status?: "alpha" | "planned"; scene: ReactNode }[] = [
  { id: "talk", scene: <TalkScene /> },
  { id: "board", status: "alpha", scene: <BoardStepScene /> },
  { id: "colab", status: "planned", scene: <ColabScene /> },
  { id: "meet", status: "planned", scene: <MeetScene /> },
];

const layerNames = ["meet", "colab", "board", "tmt core · talk · reply · receipt"];

export function Journey() {
  const { journey } = useStrings();
  const [held, setHeld] = useState(false);
  const { ref, index, choose } = useFrames<HTMLDivElement>(steps.length, {
    intervalMs: 5200,
    rest: 0,
    paused: held,
  });
  const step = steps[index];
  const text = journey.steps[index];
  // Layers grow from the foundation up to the step the reader is on.
  const lit = index;
  return (
    <div
      ref={ref}
      onPointerEnter={() => setHeld(true)}
      onPointerLeave={() => setHeld(false)}
      onFocus={() => setHeld(true)}
      onBlur={() => setHeld(false)}
      className="grid grid-cols-1 gap-x-6 gap-y-3.5 md:grid-cols-[210px_minmax(0,1fr)]"
    >
      <ol
        aria-label={journey.stepsLabel}
        className="relative m-0 grid list-none grid-cols-2 content-start gap-1.5 p-0 md:row-span-3 md:grid-cols-1"
      >
        {steps.map((item, k) => (
          <li key={item.id}>
            <button
              type="button"
              aria-pressed={k === index}
              onClick={() => choose(k)}
              className={`grid w-full cursor-pointer gap-0.5 rounded-md border-0 px-3 py-2.5 text-left font-body text-[13px] leading-snug ${
                k === index ? "bg-sheet text-text" : "bg-transparent text-muted hover:text-text"
              }`}
            >
              <b className="font-mono text-sm text-text">
                <span aria-hidden="true" className="inline-block w-[1ch] text-accent">
                  {k === index ? "›" : ""}
                </span>{" "}
                {journey.steps[k].title}
              </b>
              <span>{journey.steps[k].hint}</span>
            </button>
          </li>
        ))}
      </ol>
      <div className="flex min-h-[360px] items-center overflow-hidden rounded-xl border border-rule bg-paper p-4.5 [background-image:radial-gradient(circle_at_1px_1px,var(--c-rule)_1px,transparent_0)] [background-size:18px_18px]">
        <div
          key={step.id}
          className="w-full animate-[grow_0.45s_ease-out] motion-reduce:animate-none"
        >
          {step.scene}
        </div>
      </div>
      <div aria-label={journey.layersLabel} role="group" className="grid gap-1">
        {layerNames.map((name, k) => {
          const level = layerNames.length - 1 - k;
          const base = level === 0;
          const on = level <= lit;
          return (
            <div
              key={name}
              className={`flex justify-between gap-2.5 rounded-md border px-3 py-1.5 font-mono text-xs font-semibold transition-colors motion-reduce:transition-none ${
                base
                  ? "border-accent bg-accent text-paper"
                  : on
                    ? "border-accent bg-sheet text-text"
                    : "border-rule bg-paper text-dim"
              }`}
            >
              <span>{name}</span>
              <span className="font-body text-xs font-normal">{journey.layers[k]}</span>
            </div>
          );
        })}
      </div>
      <p className="m-0 max-w-none text-[15px] leading-normal text-muted">
        {step.status && <Tag kind={step.status}>{text.status}</Tag>} <Inline text={text.caption} />
      </p>
    </div>
  );
}
