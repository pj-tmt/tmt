import { useStrings } from "../lang/useStrings";
import "./hero-tunnel.css";

function PortalRings({ vertical = false }: { vertical?: boolean }) {
  return (
    <g
      transform={
        vertical
          ? "translate(80 152.75) rotate(90) scale(.8) translate(-245 -282)"
          : "translate(130 80) scale(.8) translate(-245 -282)"
      }
    >
      <ellipse cx="245" cy="282" rx="118" ry="66" />
      <ellipse cx="245" cy="282" rx="86" ry="48" />
      <ellipse cx="245" cy="282" rx="53" ry="30" />
    </g>
  );
}

/** Illustrative cross-harness exchange, not a live delivery indicator. */
export function HeroTunnel() {
  const { tunnel } = useStrings().landing;
  const { handoff } = useStrings().home;
  return (
    <figure className="hero-tunnel" aria-label={tunnel.label}>
      <figcaption className="tunnel-caption">{tunnel.caption}</figcaption>
      <div className="tunnel-scene">
        <article className="tunnel-terminal terminal-builder">
          <header>
            <span className="tunnel-identity">{handoff.builder}</span>
            <span>01</span>
          </header>
          <div className="tunnel-terminal-body">
            <p className="tunnel-harness">Claude Code</p>
            <p className="tunnel-context">~/project</p>
            <p className="tunnel-prompt">
              <span aria-hidden="true">❯ </span>
              {tunnel.question}
            </p>
            <div className="tunnel-returned">
              <p className="tunnel-status">
                <span aria-hidden="true">← </span>
                {handoff.reviewer}:
              </p>
              <p className="tunnel-answer">{tunnel.answer}</p>
            </div>
          </div>
        </article>
        <div className="tunnel-exchange">
          <svg
            className="tunnel-path tunnel-desktop-flow"
            viewBox="0 0 160 260"
            preserveAspectRatio="none"
            aria-hidden="true"
          >
            <PortalRings vertical />
            <path className="tunnel-route" d="M0 132.5 H160" />
            <path className="tunnel-route tunnel-return" d="M160 173 H0" />
            <path className="tunnel-arrow" d="m152 127.5 7 5-7 5 M8 168 l-7 5 7 5" />
            <rect className="tunnel-packet" x="0" y="130.5" width="4" height="4" />
            <rect className="tunnel-packet tunnel-reply" x="156" y="171" width="4" height="4" />
          </svg>
          <svg className="tunnel-path tunnel-mobile-flow" viewBox="0 0 260 160" aria-hidden="true">
            <PortalRings />
            <path className="tunnel-route" d="M110 0 V160" />
            <path className="tunnel-route tunnel-return" d="M150 160 V0" />
            <path className="tunnel-arrow" d="m105 152 5 7 5-7 M145 8 l5-7 5 7" />
            <rect className="tunnel-packet" x="108" y="0" width="4" height="4" />
            <rect className="tunnel-packet tunnel-reply" x="148" y="156" width="4" height="4" />
          </svg>
          <div className="tunnel-message tunnel-request-label">
            <span>{handoff.request}</span>
            <code>tmt talk</code>
          </div>
          <div className="tunnel-message tunnel-reply-label">
            <span>{handoff.reply}</span>
            <code>tmt reply</code>
          </div>
        </div>
        <article className="tunnel-terminal terminal-reviewer">
          <header>
            <span className="tunnel-identity">{handoff.reviewer}</span>
            <span>02</span>
          </header>
          <div className="tunnel-terminal-body">
            <p className="tunnel-harness">Codex</p>
            <p className="tunnel-context">~/project</p>
            <p className="tunnel-prompt">
              <span aria-hidden="true">› </span>
              {tunnel.received}
            </p>
            <p className="tunnel-status tunnel-submitted">
              <span aria-hidden="true">↩ </span>
              {tunnel.submitted}
            </p>
          </div>
        </article>
      </div>
      <p className="tunnel-footnote">{tunnel.note}</p>
    </figure>
  );
}
