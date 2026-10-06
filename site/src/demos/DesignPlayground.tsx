import { useId, useState, type CSSProperties } from "react";
import tokens from "../../../design/tokens/tokens.json";

type State = "ready" | "waiting" | "stopped";
const states = {
  ready: {
    mark: "●",
    title: "Ready",
    color: "working",
    body: "The example is ready for your message.",
  },
  waiting: {
    mark: "◆",
    title: "Waiting",
    color: "waiting",
    body: "Review the example before continuing. Your draft stays here.",
  },
  stopped: {
    mark: "✗",
    title: "Preview stopped",
    color: "blocked",
    body: "This example is disconnected. Your draft stays here.",
  },
} as const;

// A handbook-only presentation study, not a product client or the planned package.
export function DesignPlayground() {
  const id = useId();
  const [theme, setTheme] = useState<"light" | "dark">("light");
  const [state, setState] = useState<State>("ready");
  const [draft, setDraft] = useState("");
  const [preview, setPreview] = useState("");
  const current = states[state];
  const disabled = state !== "ready" || !draft.trim();
  const reason =
    state !== "ready"
      ? "Choose Ready to preview your draft."
      : "Enter a message to enable Preview.";
  const style = Object.fromEntries([
    ...Object.entries(tokens.color).map(([name, value]) => [`--example-${name}`, value[theme]]),
    ...Object.entries(tokens.surface).map(([name, value]) => [`--example-${name}`, value[theme]]),
  ]) as CSSProperties;
  const button =
    "border border-current aria-pressed:bg-[var(--example-accent-soft)] aria-pressed:text-[var(--example-accent)] px-3 py-2 font-mono text-xs focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-[var(--example-accent)]";
  return (
    <section
      aria-label="Interactive browser design example"
      className="my-6 min-w-0 border border-rule"
      style={style}
      data-testid="design-playground"
    >
      <div className="flex flex-wrap gap-5 bg-[var(--example-paper)] p-4 text-[var(--example-text)]">
        <div role="group" aria-label="Example theme" className="flex flex-wrap items-center gap-2">
          <span className="mr-1 font-mono text-xs">Theme</span>
          {(["light", "dark"] as const).map((value) => (
            <button
              key={value}
              className={button}
              aria-pressed={theme === value}
              onClick={() => setTheme(value)}
            >
              {value === "light" ? "Light" : "Dark"}
            </button>
          ))}
        </div>
        <div role="group" aria-label="Example state" className="flex flex-wrap items-center gap-2">
          <span className="mr-1 font-mono text-xs">State</span>
          {(Object.keys(states) as State[]).map((value) => (
            <button
              key={value}
              className={button}
              aria-pressed={state === value}
              onClick={() => setState(value)}
            >
              {value === "stopped" ? "Stopped" : states[value].title}
            </button>
          ))}
        </div>
      </div>
      <div className="border-t border-[var(--example-rule)] bg-[var(--example-sheet)] text-[var(--example-text)]">
        <header className="flex min-h-12 flex-wrap items-center gap-3 border-b border-[var(--example-rule)] px-4 py-3">
          <span className="font-mono text-xs font-semibold text-[var(--example-muted)]">tmt</span>
          <strong className="text-sm">Browser example</strong>
          <span className="ml-auto text-sm" style={{ color: tokens.color[current.color][theme] }}>
            <span aria-hidden="true">{current.mark}</span> {current.title}
          </span>
        </header>
        <div className="p-5 sm:p-8">
          <div
            className="border-2 border-current p-5 sm:p-7"
            style={{ boxShadow: `4px 4px 0 ${tokens.color[current.color][theme]}` }}
          >
            <div aria-live="polite" aria-atomic="true">
              <span aria-hidden="true" style={{ color: tokens.color[current.color][theme] }}>
                {current.mark}
              </span>
              <h4 className="mt-2 mb-3 font-display text-2xl font-bold">{current.title}</h4>
              <p>{current.body}</p>
            </div>
            <form
              className="mt-6"
              onSubmit={(event) => {
                event.preventDefault();
                if (!disabled) setPreview(draft);
              }}
            >
              <label className="mb-2 block text-sm font-semibold" htmlFor={`${id}-draft`}>
                Example message
              </label>
              <input
                id={`${id}-draft`}
                value={draft}
                onChange={(event) => setDraft(event.target.value)}
                aria-describedby={`${id}-hint`}
                className="w-full min-w-0 border border-current bg-[var(--example-paper)] px-3 py-2 text-base focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--example-accent)]"
              />
              <p id={`${id}-hint`} className="mt-2 text-sm text-[var(--example-muted)]">
                {disabled ? reason : "Preview only: this example does not send a request."}
              </p>
              <button
                type="submit"
                disabled={disabled}
                className={`${button} mt-3 disabled:cursor-not-allowed disabled:opacity-50`}
              >
                Preview
              </button>
              {preview && (
                <output className="mt-4 block break-words border-t border-[var(--example-rule)] pt-3 text-sm">
                  Local preview: {preview}
                </output>
              )}
            </form>
          </div>
        </div>
      </div>
      <p className="m-0 border-t border-rule p-4 text-sm text-muted">
        Design example · local state only · shared component package planned
      </p>
    </section>
  );
}
