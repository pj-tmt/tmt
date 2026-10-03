import { useStrings } from "../lang/useStrings";

export type SpatialStageProps = { frame: number };

// CSS perspective projects a single spatial model. The request and reply
// cross its exchange plane; the ledger stays at the same durable owner.
export function SpatialStage({ frame }: SpatialStageProps) {
  const { handoff } = useStrings().home;
  return (
    <div className="exchange-stage" role="img" aria-label={handoff.label}>
      <div className="exchange-world" aria-hidden="true">
        <div className="exchange-ground">
          <span>{handoff.local}</span>
        </div>
        <svg className="exchange-routes" viewBox="0 0 600 340">
          <path d="M100 120 L100 225 L300 225 L300 135 L500 135" />
          <path className="route-return" d="M500 180 L500 280 L100 280 L100 160" />
        </svg>
        <div
          className={`spatial-pane spatial-reviewer ${frame === 0 || frame === 3 ? "pane-active" : ""}`}
        >
          <div className="spatial-pane-bar">
            <i />
            {handoff.reviewer}
            <span>Claude Code</span>
          </div>
          <div className="spatial-pane-body">
            <b>$ tmt talk builder</b>
            <span>"Review the diff"</span>
            <hr />
            <span className={frame === 3 ? "spatial-success" : "spatial-muted"}>
              {frame === 3 ? "✓ No blocking issues" : "waiting for a reply…"}
            </span>
          </div>
        </div>
        <div className="spatial-ledger">
          <span>tmt</span>
          <b>req_9ba4…</b>
          <code>{handoff.receipt} v2_…</code>
          <i>{frame === 3 ? "✓ result stored" : "request stored"}</i>
        </div>
        <div className={`spatial-pane spatial-builder ${frame === 2 ? "pane-active" : ""}`}>
          <div className="spatial-pane-bar">
            <i />
            {handoff.builder}
            <span>Codex</span>
          </div>
          <div className="spatial-pane-body">
            <span className="spatial-muted">req_9ba4… / v2_…</span>
            <b>$ tmt reply req_9ba4…</b>
            <span>--receipt v2_…</span>
            <span className="spatial-success">--message "No blocking issues"</span>
          </div>
        </div>
        <div className={`spatial-packet packet-${frame}`}>
          <span>{frame < 2 ? "→" : "←"}</span>
          {frame === 0
            ? handoff.request
            : frame === 1
              ? handoff.receipt
              : frame === 2
                ? handoff.reply
                : `✓ ${handoff.result}`}
        </div>
      </div>
      <span className="stage-coordinate">X / {handoff.request}</span>
      <span className="stage-coordinate stage-coordinate-right">Y / {handoff.receipt}</span>
    </div>
  );
}
