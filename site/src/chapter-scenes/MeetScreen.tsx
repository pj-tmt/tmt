import { useStrings } from "../lang/useStrings";
import { Window } from "../scenes/Window";

const PEOPLE = ["you", "lead", "builder", "reviewer", "tester"];

export type FloorMode = "host" | "first";

// One frame of the planned terminal meeting: who has the floor (●), the raised
// hands (↑, numbered in first-come mode), who is in the room and the floor
// mode. It is shared by the Meet page and the start journey. Pass `onMode` to
// make the floor-mode switch clickable; without it the switch is a label.
export function MeetScreen({
  index,
  mode,
  onMode,
}: {
  index: number;
  mode: FloorMode;
  onMode?: (mode: FloorMode) => void;
}) {
  const { meet } = useStrings().chapters;
  const turn = meet.script[index];
  const log = meet.script.slice(Math.max(0, index - 2), index);
  const heading = "m-0 mb-1 text-[11px] font-semibold tracking-[0.08em] text-t-muted uppercase";
  return (
    <Window
      title={meet.title}
      footer={
        <div className="bg-term-bar px-3 py-1.5 font-mono text-[11px] text-t-dim">
          {meet.keys} · {meet.sketch}
        </div>
      }
    >
      <div className="grid grid-cols-1 gap-px bg-term-edge md:grid-cols-[minmax(0,1fr)_240px]">
        <div className="min-w-0 bg-term p-4 font-mono text-[13px] leading-[1.6] text-t-text">
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
          <h4 className={heading}>{meet.inRoom}</h4>
          <div className="mb-3 grid gap-0.5">
            {PEOPLE.map((name) => (
              <div key={name} className={name === turn.who ? "bg-t-selection px-1" : "px-1"}>
                {name === turn.who ? <span className="text-t-working">● </span> : "  "}
                {name}
              </div>
            ))}
          </div>
          <h4 className={heading}>{meet.hands}</h4>
          <div className="mb-3 grid gap-0.5">
            {turn.hands.length === 0 && <span className="text-t-dim">{meet.noHands}</span>}
            {turn.hands.map((name, k) => (
              <div key={name} className="px-1">
                <span className="text-t-waiting">↑{mode === "first" ? k + 1 : ""}</span> {name}
              </div>
            ))}
          </div>
          <h4 className={heading}>{meet.floor}</h4>
          <div role="group" aria-label={meet.floor} className="flex gap-1">
            {(["host", "first"] as const).map((choice) => {
              const on = mode === choice;
              const look = on
                ? "border-(--t-text) bg-t-text text-term-bar"
                : "border-term-edge bg-transparent text-t-muted";
              const box = `flex-1 border px-1.5 py-1.5 text-center text-[11px] font-semibold ${look}`;
              return onMode ? (
                <button
                  key={choice}
                  type="button"
                  aria-pressed={on}
                  onClick={() => onMode(choice)}
                  className={`${box} cursor-pointer`}
                >
                  {meet[choice]}
                </button>
              ) : (
                <span key={choice} className={box}>
                  {meet[choice]}
                </span>
              );
            })}
          </div>
        </div>
      </div>
    </Window>
  );
}
