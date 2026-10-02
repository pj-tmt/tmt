import { useState } from "react";
import { useStrings } from "../lang/useStrings";
import { useFrames } from "../scenes/useFrames";
import { MeetScreen, type FloorMode } from "./MeetScreen";

// The Meet page's sketch: four turns of the script, cycling while on screen.
// Under reduced motion it rests on the first turn, with two hands raised.
export function MeetRoom() {
  const { meet } = useStrings().chapters;
  const [mode, setMode] = useState<FloorMode>("host");
  const { ref, index } = useFrames<HTMLDivElement>(meet.script.length, {
    intervalMs: 3600,
    rest: 0,
  });
  return (
    <div ref={ref} role="group" aria-label={meet.label} className="my-5">
      <MeetScreen index={index} mode={mode} onMode={setMode} />
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
