import "./hero-tunnel.css";

const routes = [
  "M130 156 C110 250 350 212 360 400",
  "M380 226 C350 285 90 255 102 389",
  "M112 390 C40 300 42 205 130 156",
  "M360 400 C470 325 480 272 380 226",
  "M102 420 C180 510 290 475 360 420",
];

/** Illustrative cross-harness exchange, not a live delivery indicator. */
export function HeroTunnel() {
  return (
    <div
      className="hero-tunnel"
      aria-label="Claude Code, Codex, Pi and Cursor exchange requests across terminals and machines"
    >
      <div className="tunnel-caption">DIFFERENT HARNESSES. YOUR OWN ENVIRONMENT.</div>
      <svg className="tunnel-path" viewBox="0 0 480 560" aria-hidden="true">
        <ellipse cx="245" cy="282" rx="118" ry="66" />
        <ellipse cx="245" cy="282" rx="86" ry="48" />
        <ellipse cx="245" cy="282" rx="53" ry="30" />
        {routes.map((path, index) => (
          <g key={path}>
            <path d={path} className={index > 1 ? "tunnel-return" : ""} />
            <circle
              className={`tunnel-packet ${index % 2 ? "tunnel-reply" : ""}`}
              r={index < 2 ? 4 : 3}
            >
              <animateMotion
                dur={`${5 + index}s`}
                begin={`${-index * 1.7}s`}
                repeatCount="indefinite"
                path={path}
              />
            </circle>
          </g>
        ))}
      </svg>
      <article className="tunnel-terminal terminal-reviewer">
        <header>
          <span>›_ Claude Code</span>
          <span>01</span>
        </header>
        <div className="tunnel-environment">
          <span>tmux</span>
          <span>remote devbox</span>
        </div>
        <div className="tunnel-terminal-body">
          <p className="tunnel-command">$ tmt this reviewer</p>
          <p className="tunnel-label">↓ REQUEST FROM BUILDER</p>
          <p>Review this patch?</p>
        </div>
      </article>
      <article className="tunnel-terminal terminal-pi">
        <header>
          <span>›_ Pi agent</span>
          <span>02</span>
        </header>
        <div className="tunnel-environment">
          <span>Herdr</span>
          <span>local</span>
        </div>
        <div className="tunnel-terminal-body">
          <p>↗ Context shared</p>
          <div className="tunnel-code-lines">
            <i />
            <i />
          </div>
        </div>
      </article>
      <div className="tunnel-message">
        <span>↗ request</span>
        <span>↙ reply</span>
      </div>
      <article className="tunnel-terminal terminal-cursor">
        <header>
          <span>›_ Cursor</span>
          <span>03</span>
        </header>
        <div className="tunnel-environment">
          <span>terminal</span>
          <span>another machine</span>
        </div>
        <div className="tunnel-terminal-body">
          <p>✓ Checks passed</p>
          <div className="tunnel-code-lines">
            <i />
            <i />
          </div>
        </div>
      </article>
      <article className="tunnel-terminal terminal-builder">
        <header>
          <span>›_ Codex</span>
          <span>04</span>
        </header>
        <div className="tunnel-environment">
          <span>iTerm</span>
          <span>local laptop</span>
        </div>
        <div className="tunnel-terminal-body">
          <p className="tunnel-command">$ tmt talk reviewer …</p>
          <p className="tunnel-label">✓ REPLY RETURNED</p>
          <p>Ready for the next step.</p>
        </div>
      </article>
      <div className="tunnel-footnote">
        <span>YOUR TERMINALS · YOUR CONTEXT</span>
        <span className="tunnel-remote-note">
          Illustrative exchange · remote via optional extension
        </span>
      </div>
    </div>
  );
}
