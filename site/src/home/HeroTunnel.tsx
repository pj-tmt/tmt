import { useStrings } from "../lang/useStrings";
import "./hero-tunnel.css";

/** Illustrative cross-harness exchange, not a live delivery indicator. */
export function HeroTunnel() {
  const { tunnel } = useStrings().landing;
  const { handoff } = useStrings().home;
  return (
    <figure className="hero-tunnel" aria-label={tunnel.label}>
      <figcaption className="tunnel-caption">{tunnel.caption}</figcaption>
      <div className="tunnel-terminals">
        <article className="tunnel-terminal terminal-reviewer">
          <header>
            <span className="tunnel-identity">{handoff.reviewer}</span>
            <span>01</span>
          </header>
          <div className="tunnel-terminal-body">
            <p className="tunnel-harness">Claude Code</p>
            <p className="tunnel-context">~/project</p>
            <p className="tunnel-prompt">
              <span aria-hidden="true">❯ </span>
              {tunnel.question}
            </p>
          </div>
        </article>
        <article className="tunnel-terminal terminal-builder">
          <header>
            <span className="tunnel-identity">{handoff.builder}</span>
            <span>02</span>
          </header>
          <div className="tunnel-terminal-body">
            <p className="tunnel-harness">Codex</p>
            <p className="tunnel-context">~/project</p>
            <p className="tunnel-prompt">
              <span aria-hidden="true">› </span>
              {tunnel.answer}
            </p>
          </div>
        </article>
      </div>
      <div className="tunnel-exchange">
        <svg className="tunnel-path" viewBox="0 0 480 180" aria-hidden="true">
          <g transform="translate(0 -192)">
            <ellipse cx="245" cy="282" rx="118" ry="66" />
            <ellipse cx="245" cy="282" rx="86" ry="48" />
            <ellipse cx="245" cy="282" rx="53" ry="30" />
          </g>
          <path className="tunnel-route" d="M28 0 V62 H452 V0" />
          <path className="tunnel-route tunnel-return" d="M452 0 V120 H28 V0" />
          <path className="tunnel-arrow" d="m440 57 7 5-7 5 M40 115 l-7 5 7 5" />
          <rect className="tunnel-packet" x="26" y="60" width="4" height="4" />
          <rect className="tunnel-packet tunnel-reply" x="450" y="118" width="4" height="4" />
        </svg>
        <div className="tunnel-message tunnel-request-label">
          <span>
            {handoff.request} <span aria-hidden="true">→</span>
          </span>
          <code>tmt talk</code>
        </div>
        <div className="tunnel-message tunnel-reply-label">
          <span>
            <span aria-hidden="true">←</span> {handoff.reply}
          </span>
          <code>tmt reply</code>
        </div>
      </div>
      <p className="tunnel-footnote">{tunnel.note}</p>
    </figure>
  );
}
