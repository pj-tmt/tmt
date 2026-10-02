import { useState, type ReactNode } from "react";
import { Tag } from "../components/marks";
import { useFrames } from "../scenes/useFrames";
import { BoardStepScene, ColabScene, MeetScene, TalkScene } from "./journey-scenes";

type Step = {
  id: string;
  title: string;
  hint: string;
  status?: "alpha" | "in progress" | "planned";
  scene: ReactNode;
  caption: ReactNode;
};

// Each step adds one layer on the same foundation: stopping at any step leaves
// something that works. Only the first two are released; the others say so.
const steps: Step[] = [
  {
    id: "talk",
    title: "1 · talk",
    hint: "Two panes, one message",
    scene: <TalkScene />,
    caption: (
      <>
        The smallest start: two agents in two panes, and one sends the other a request with{" "}
        <code>tmt talk</code>. The reply comes back with a receipt, and everything below builds on
        exactly this.
      </>
    ),
  },
  {
    id: "board",
    title: "2 · board",
    hint: "The whole team in one view",
    status: "alpha",
    scene: <BoardStepScene />,
    caption: (
      <>
        More agents? <code>tmt sq board</code> shows the whole team in one view: who works, who
        waits on you, who is blocked. It comes with the Squad extension. The same agents, one more
        window.
      </>
    ),
  },
  {
    id: "colab",
    title: "3 · colab",
    hint: "One shared page",
    status: "in progress",
    scene: <ColabScene />,
    caption: (
      <>
        Colab is a shared page for the lead&apos;s plan, notes and discussion, so teammates and
        agents on other machines read and comment on the same page. What you see is the design, not
        a release.
      </>
    ),
  },
  {
    id: "meet",
    title: "4 · meet",
    hint: "Meet with your agents",
    status: "planned",
    scene: <MeetScene />,
    caption: (
      <>
        Meet puts you, the lead and its members in one text meeting. Whoever wants to speak raises a
        hand, and you give the floor. Not started.
      </>
    ),
  },
];

const layers = [
  { name: "meet", text: "a meeting room · planned" },
  { name: "colab", text: "a shared page · in progress" },
  { name: "board", text: "the team in one view · alpha" },
  { name: "tmt core · talk · reply · receipt", text: "the foundation every layer uses" },
];

export function Journey() {
  const [held, setHeld] = useState(false);
  const { ref, index, choose } = useFrames<HTMLDivElement>(steps.length, {
    intervalMs: 5200,
    rest: 0,
    paused: held,
  });
  const step = steps[index];
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
        aria-label="Steps"
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
                <span className={k === index ? "text-accent" : "text-dim"}>
                  {k < index ? "●" : k === index ? "◆" : "○"}
                </span>{" "}
                {item.title}
              </b>
              <span>{item.hint}</span>
            </button>
          </li>
        ))}
      </ol>
      <div className="min-h-[360px] overflow-hidden rounded-xl border border-rule bg-paper p-4.5 [background-image:radial-gradient(circle_at_1px_1px,var(--c-rule)_1px,transparent_0)] [background-size:18px_18px]">
        <div key={step.id} className="animate-[grow_0.45s_ease-out] motion-reduce:animate-none">
          {step.scene}
        </div>
      </div>
      <div aria-label="Layers on one foundation" role="group" className="grid gap-1">
        {layers.map((layer, k) => {
          const level = layers.length - 1 - k;
          const base = level === 0;
          const on = level <= lit;
          return (
            <div
              key={layer.name}
              className={`flex justify-between gap-2.5 rounded-md border px-3 py-1.5 font-mono text-xs font-semibold transition-colors motion-reduce:transition-none ${
                base
                  ? "border-accent bg-accent text-paper"
                  : on
                    ? "border-accent bg-sheet text-text"
                    : "border-rule bg-paper text-dim"
              }`}
            >
              <span>{layer.name}</span>
              <span className="font-body text-xs font-normal">{layer.text}</span>
            </div>
          );
        })}
      </div>
      <p className="m-0 max-w-none text-[15px] leading-normal text-muted">
        {step.status && (
          <Tag kind={step.status === "alpha" ? "alpha" : "planned"}>{step.status}</Tag>
        )}{" "}
        {step.caption}
      </p>
    </div>
  );
}
