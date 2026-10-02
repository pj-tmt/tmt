import { useState } from "react";
import { useStrings } from "../lang/useStrings";
import { useFrames } from "../scenes/useFrames";
import { Window } from "../scenes/Window";

const PEOPLE = ["you", "lead", "builder", "reviewer", "tester"];

// A sketch of the terminal meeting. The script is four turns; whoever has the
// floor is marked ●, and a raised hand waits on you ◆. The floor-mode switch
// changes how the queue is labelled. Under reduced motion it rests on the
// first turn, with two hands raised.
export function MeetRoom() {
  const { meet } = useStrings().chapters;
  const [mode, setMode] = useState<"host" | "first">("host");
  const { ref, index } = useFrames<HTMLDivElement>(meet.script.length, {
    intervalMs: 3600,
    rest: 0,
  });
  const turn = meet.script[index];
  const log = meet.script.slice(Math.max(0, index - 2), index);
  return (
    <div ref={ref} role="group" aria-label={meet.label} className="my-5">
      <Window
        title={meet.title}
        footer={
          <div className="bg-term-bar px-3 py-1.5 font-mono text-[11px] text-t-dim">
            {meet.keys} · {meet.sketch}
          </div>
        }
      >
        <div className="grid grid-cols-1 gap-px bg-term-edge md:grid-cols-[minmax(0,1fr)_240px]">
          <div className="min-w-0 bg-term p-4 font-mono text-[13px] leading-[1.6]">
            <div>
              <span className="text-t-working">●</span> <b>{turn.who}</b>{" "}
              <span className="text-t-muted">{meet.hasFloor}</span>
            </div>
            <div className="mt-1 min-h-[3.2em] pl-4 break-words">{turn.say}</div>
            <div className="mt-3 text-t-muted">
              {log.map((entry) => (
                <div key={entry.say} className="truncate">
                  <b className="text-t-text">{entry.who}</b> {entry.say}
                </div>
              ))}
            </div>
          </div>
          <div className="bg-term p-4 font-mono text-xs leading-[1.6] text-t-text">
            <div>
              <h4 className="m-0 mb-1 text-[11px] font-semibold tracking-[0.08em] text-t-muted uppercase">
                {meet.inRoom}
              </h4>
              <div className="mb-3 grid gap-0.5">
                {PEOPLE.map((name) => (
                  <div key={name} className={name === turn.who ? "bg-t-selection px-1" : "px-1"}>
                    {name === turn.who ? <span className="text-t-working">● </span> : "  "}
                    {name}
                  </div>
                ))}
              </div>
              <h4 className="m-0 mb-1 text-[11px] font-semibold tracking-[0.08em] text-t-muted uppercase">
                {meet.hands}
              </h4>
              <div className="mb-3 grid gap-0.5">
                {turn.hands.length === 0 && <span className="text-t-dim">{meet.noHands}</span>}
                {turn.hands.map((name, k) => (
                  <div key={name} className="flex gap-2 px-1">
                    <span className="text-t-waiting">◆</span>
                    {name}
                    <span className="ml-auto text-t-dim">
                      {mode === "first" ? `#${k + 1}` : meet.wait}
                    </span>
                  </div>
                ))}
              </div>
            </div>
            <h4 className="m-0 mb-1 text-[11px] font-semibold tracking-[0.08em] text-t-muted uppercase">
              {meet.floor}
            </h4>
            <div role="group" aria-label={meet.floor} className="flex gap-1">
              {(["host", "first"] as const).map((choice) => (
                <button
                  key={choice}
                  type="button"
                  aria-pressed={mode === choice}
                  onClick={() => setMode(choice)}
                  className={`flex-1 cursor-pointer rounded border px-1.5 py-1.5 text-[11px] font-semibold ${
                    mode === choice
                      ? "border-(--t-text) bg-t-text text-term-bar"
                      : "border-term-edge bg-transparent text-t-muted"
                  }`}
                >
                  {meet[choice]}
                </button>
              ))}
            </div>
          </div>
        </div>
      </Window>
      <ol className="m-0 mt-3.5 grid list-none grid-cols-1 gap-3 p-0 md:grid-cols-3">
        {meet.roadmap.map((step, k) => (
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
