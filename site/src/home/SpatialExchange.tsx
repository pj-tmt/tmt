import { createElement, useRef, useState } from "react";
import { useStrings } from "../lang/useStrings";
import { useFrames } from "../scenes/useFrames";
import { useSpatialStage } from "../scenes/useSpatialStage";
import "./spatial-exchange.css";

const loadStage = () => import("./SpatialStage").then((module) => module.SpatialStage);

// The flat diagram is useful content while the optional scene loads, on a
// phone, under reduced motion, or when the scene module cannot be fetched.
function FlatExchange() {
  const { handoff } = useStrings().home;
  return (
    <div className="exchange-flat" role="img" aria-label={handoff.label}>
      <div className="flat-agent">
        <b>{handoff.reviewer}</b>
        <span>Claude Code</span>
      </div>
      <div className="flat-flow">
        <span>01 · {handoff.request} →</span>
        <b>tmt</b>
        <code>req_9ba4… / v2_…</code>
        <span>← 03 · {handoff.reply}</span>
      </div>
      <div className="flat-agent">
        <b>{handoff.builder}</b>
        <span>Codex</span>
      </div>
      <p>02 · {handoff.tied}</p>
    </div>
  );
}

export function SpatialExchange() {
  const ref = useRef<HTMLDivElement>(null);
  const Stage = useSpatialStage(ref, loadStage);
  const [paused, setPaused] = useState(false);
  const [selected, setSelected] = useState<number | null>(null);
  const { index } = useFrames<HTMLDivElement>(4, {
    ref,
    intervalMs: 2600,
    rest: 3,
    paused: paused || selected !== null || !Stage,
  });
  const { handoff } = useStrings().home;
  const frame = selected ?? index;
  const live = Stage !== null;
  const steps = [handoff.request, handoff.receipt, handoff.reply, handoff.result];
  return (
    <div ref={ref} className="spatial-exchange" data-frame={frame}>
      <div className="exchange-heading">
        <span>{handoff.title}</span>
        <span>{live ? handoff.spatial : handoff.flat}</span>
      </div>
      {live && Stage ? createElement(Stage, { frame }) : <FlatExchange />}
      <div className="exchange-controls">
        <ol aria-label={handoff.steps}>
          {steps.map((step, k) => (
            <li key={step}>
              <button
                type="button"
                onClick={() => setSelected(k)}
                aria-pressed={live && frame === k}
                disabled={!live}
              >
                <span>0{k + 1}</span>
                {step}
              </button>
            </li>
          ))}
        </ol>
        {live && (
          <button
            type="button"
            className="exchange-play"
            aria-label={selected !== null || paused ? handoff.play : handoff.pause}
            onClick={() => {
              if (selected !== null) {
                setSelected(null);
                setPaused(false);
              } else setPaused(!paused);
            }}
          >
            {selected !== null || paused ? `▶ ${handoff.play}` : `Ⅱ ${handoff.pause}`}
          </button>
        )}
      </div>
      <p className="exchange-note">{handoff.note}</p>
    </div>
  );
}
