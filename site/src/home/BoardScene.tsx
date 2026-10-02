import { boardLines, type BoardSpec, type Row } from "../demos/board";
import { FitWidth } from "../demos/FitWidth";
import { TmuxBar, Window } from "../scenes/Window";
import { useFrames } from "../scenes/useFrames";

const builder = (state: string, task: string, pr: string): Row => ["builder", state, task, pr];
const reviewer = (state: string, task: string, waiting?: boolean): Row => [
  "reviewer",
  state,
  task,
  "#408",
  waiting,
];
const tester = (state: string, task: string): Row => ["tester", state, task, ""];

// One complete board per frame: reviewer starts a review, asks the human a
// question, then the answer unblocks the others. The last frame is the one
// shown when motion is reduced.
const frames: BoardSpec[] = [
  {
    rows: [
      builder("working", "rotate tokens", "#412"),
      reviewer("review", "auth flow"),
      tester("idle", ""),
    ],
    sel: 1,
  },
  {
    rows: [
      builder("working", "rotate tokens", "#412"),
      reviewer("review", "auth flow", true),
      tester("idle", ""),
    ],
    sel: 1,
    pending: "ship #412 now?",
  },
  {
    rows: [
      builder("working", "rotate tokens", "#412"),
      reviewer("review", "auth flow", true),
      tester("working", "e2e on #412"),
    ],
    sel: 1,
    pending: "ship #412 now?",
  },
  {
    rows: [
      builder("idle", "", "#412"),
      reviewer("working", "auth flow"),
      tester("working", "e2e on #412"),
    ],
    sel: 1,
  },
];
const REST = 1;

const LABEL =
  "A tmt sq board that updates in four steps: builder works on rotating tokens, reviewer starts a review and then waits on you to decide whether to ship, tester starts end-to-end tests, and once you answer the others carry on.";

export function BoardScene() {
  const { ref, index } = useFrames<HTMLDivElement>(frames.length, {
    intervalMs: 2600,
    rest: REST,
  });
  const frame = frames[index];
  const waiting = frame.rows.some((row) => row[4]);
  return (
    <div ref={ref}>
      <Window
        title="tmt sq board"
        label={LABEL}
        footer={<TmuxBar window="2:squad*" message={waiting ? "◆ reviewer waits on you" : null} />}
      >
        <div aria-hidden="true" className="px-3.5 py-3">
          <FitWidth width={560}>
            <pre className="m-0 font-mono text-[12.5px] leading-[1.6]">
              {boardLines(frame, "", 0, false)}
            </pre>
          </FitWidth>
        </div>
      </Window>
    </div>
  );
}
