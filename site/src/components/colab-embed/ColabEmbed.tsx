import { Check, MousePointer2, PanelRightOpen, RotateCcw, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import "./colab-embed.css";
import { isColabShortcut, isImeConfirmation } from "./keyboard";

type Anchor = { path: string; quote: string; page: string };
type Turn = {
  id: string;
  author: string;
  body: string;
  request?: string;
  state?: string;
  recipient?: string;
};
type Thread = { id: string; anchor: Anchor; turns: Turn[]; draft: string; resolved: boolean };
type Agent = { id: string; name: string; presence: string };
const storageKey = "tmt.handbook.embed.v1";
function load(): Thread[] {
  try {
    const parsed = JSON.parse(localStorage.getItem(storageKey) ?? "[]");
    return Array.isArray(parsed)
      ? parsed.filter(
          (t) =>
            typeof t.id === "string" &&
            typeof t.anchor?.path === "string" &&
            Array.isArray(t.turns) &&
            typeof t.draft === "string",
        )
      : [];
  } catch {
    return [];
  }
}
function elementPath(element: Element): string {
  if (element.id) return `#${CSS.escape(element.id)}`;
  const parts: string[] = [];
  let current: Element | null = element;
  while (current && current !== document.body) {
    if (current.id) {
      parts.unshift(`#${CSS.escape(current.id)}`);
      break;
    }
    const siblings: Element[] = current.parentElement
      ? [...current.parentElement.children].filter((node) => node.tagName === current!.tagName)
      : [];
    parts.unshift(`${current.tagName.toLowerCase()}:nth-of-type(${siblings.indexOf(current) + 1})`);
    current = current.parentElement;
  }
  return parts.join(" > ");
}
function resolve(anchor: Anchor): Element | null {
  if (anchor.page !== location.pathname) return null;
  try {
    const node = document.querySelector(anchor.path);
    return node &&
      (!anchor.quote ||
        node.textContent?.replace(/\s/g, "").includes(anchor.quote.replace(/\s/g, "")))
      ? node
      : null;
  } catch {
    return null;
  }
}
async function api(path: string, data?: unknown): Promise<Record<string, unknown>> {
  const response = await fetch(`/__tmt_embed/${path}`, {
    method: data ? "POST" : "GET",
    headers: { "X-Tmt-Embed": "1", ...(data ? { "Content-Type": "application/json" } : {}) },
    body: data ? JSON.stringify(data) : undefined,
  });
  const value = await response.json();
  if (!response.ok || (value.error && value.status !== "unavailable"))
    throw new Error(
      typeof value.error === "string" ? value.error : "TMT could not complete this action",
    );
  return value;
}
export function ColabEmbed() {
  const [threads, setThreads] = useState<Thread[]>(load);
  const [mode, setMode] = useState(false);
  const [setup, setSetup] = useState(false);
  const [active, setActive] = useState<string | null>(null);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [agent, setAgent] = useState(localStorage.getItem(`${storageKey}.agent`) ?? "");
  const [remote, setRemote] = useState<string | null>(null);
  const [status, setStatus] = useState("◐ Connecting");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [positions, setPositions] = useState<Record<string, { top: number; left: number }>>({});
  const [hover, setHover] = useState<{
    top: number;
    left: number;
    width: number;
    height: number;
  } | null>(null);
  const [choice, setChoice] = useState(false);
  const fab = useRef<HTMLButtonElement>(null);
  const composer = useRef<HTMLTextAreaElement>(null);
  const messages = useRef<HTMLDivElement>(null);
  // Consume a draft synchronously: React's busy render can follow another key event.
  const submission = useRef<{
    key: string;
    operationId: string;
    message: string;
    pending: boolean;
    done: boolean;
  } | null>(null);
  const toggleMode = useCallback(() => {
    if (busy || submission.current?.pending) return;
    setMode((enabled) => !enabled);
    setActive(null);
    setSetup(false);
    setHover(null);
  }, [busy]);
  useEffect(() => {
    const shortcut = (event: KeyboardEvent) => {
      const target = event.target;
      const editing =
        target instanceof HTMLElement &&
        (target.isContentEditable || !!target.closest('input, textarea, select, [role="textbox"]'));
      if (event.defaultPrevented || !isColabShortcut(event, editing)) return;
      event.preventDefault();
      toggleMode();
    };
    document.addEventListener("keydown", shortcut);
    return () => document.removeEventListener("keydown", shortcut);
  }, [toggleMode]);
  const thread = threads.find((item) => item.id === active);
  useEffect(() => {
    const viewport = messages.current;
    if (viewport) viewport.scrollTop = viewport.scrollHeight;
  }, [active, thread?.turns]);
  const update = (id: string, change: Partial<Thread>) =>
    setThreads((items) => items.map((item) => (item.id === id ? { ...item, ...change } : item)));
  useEffect(() => {
    try {
      localStorage.setItem(storageKey, JSON.stringify(threads));
    } catch {
      queueMicrotask(() =>
        setError("Browser storage is full. Keep this page open to retain your discussion."),
      );
    }
  }, [threads]);
  useEffect(() => {
    localStorage.setItem(`${storageKey}.agent`, agent);
  }, [agent]);
  const connected = (value: Record<string, unknown>) => {
    const list = (value.agents as { identities: Agent[] }).identities;
    setAgents(list);
    setAgent((previous) =>
      list.some((item) => item.id === previous)
        ? previous
        : (list.find((item) => item.name === "tmt-mkt-lead")?.id ?? ""),
    );
    const door = value.remote as { running?: boolean; origin?: string; path?: string } | null;
    setRemote(door?.running && door.origin && door.path ? `${door.origin}${door.path}/` : null);
    setStatus("● Local developer connection");
  };
  const connectionFailed = (fault: unknown) => {
    setStatus("✗ Disconnected");
    setError(fault instanceof Error ? fault.message : String(fault));
  };
  const connect = () => api("status").then(connected, connectionFailed);
  useEffect(() => {
    void api("status").then(connected, connectionFailed);
  }, []);
  useEffect(() => {
    if (active) composer.current?.focus({ preventScroll: true });
  }, [active]);
  useEffect(() => {
    if (!mode) return;
    let frame = 0;
    const layout = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const next: Record<string, { top: number; left: number }> = {};
        const occupied: number[] = [];
        for (const item of threads) {
          const node = resolve(item.anchor);
          if (!node) continue;
          const rect = node.getBoundingClientRect();
          let top = rect.top + window.scrollY;
          while (occupied.some((y) => Math.abs(y - top) < 32)) top += 32;
          occupied.push(top);
          next[item.id] = {
            top,
            left: Math.max(8, Math.min(window.innerWidth - 48, rect.right + 8)),
          };
        }
        setPositions(next);
      });
    };
    layout();
    const observer = new MutationObserver(layout);
    const root = document.getElementById("root");
    if (root) observer.observe(root, { childList: true, subtree: true, characterData: true });
    window.addEventListener("resize", layout);
    window.addEventListener("scroll", layout, { passive: true });
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
      window.removeEventListener("resize", layout);
      window.removeEventListener("scroll", layout);
    };
  }, [mode, threads]);
  useEffect(() => {
    if (!mode) return;
    const select = (event: MouseEvent) => {
      const target = event.target instanceof Element ? event.target : null;
      if (!target || target.closest("#tmt-colab-embed") || busy) return;
      const selection = window.getSelection();
      const quote = selection?.toString().trim().slice(0, 2000) ?? "";
      if (!quote && !event.altKey && active) {
        setActive(null);
        setSetup(false);
        return;
      }
      let node = quote && selection?.anchorNode ? selection.anchorNode.parentElement : target;
      if (!node || !document.getElementById("root")?.contains(node)) return;
      if (quote && !node.textContent?.replace(/\s/g, "").includes(quote.replace(/\s/g, "")))
        node =
          selection?.getRangeAt(0).commonAncestorContainer instanceof Element
            ? (selection.getRangeAt(0).commonAncestorContainer as Element)
            : node;
      const anchor = { path: elementPath(node), quote, page: location.pathname };
      const existing = threads.find(
        (item) => item.anchor.path === anchor.path && item.anchor.quote === quote,
      );
      if (existing) setActive(existing.id);
      else {
        const item: Thread = {
          id: crypto.randomUUID(),
          anchor,
          turns: [],
          draft: "",
          resolved: false,
        };
        setThreads((items) => [...items, item]);
        setActive(item.id);
      }
      setSetup(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) {
        setActive(null);
        setSetup(false);
        fab.current?.focus();
      }
    };
    const point = (event: MouseEvent) => {
      const target = event.target instanceof Element ? event.target : null;
      if (!target || !document.getElementById("root")?.contains(target)) {
        setHover(null);
        return;
      }
      const rect = target.getBoundingClientRect();
      setHover({ top: rect.top, left: rect.left, width: rect.width, height: rect.height });
    };
    const contain = (event: MouseEvent) => {
      if (
        event.target instanceof Element &&
        document.getElementById("root")?.contains(event.target)
      ) {
        event.preventDefault();
        event.stopPropagation();
      }
    };
    document.addEventListener("mousemove", point);
    document.addEventListener("click", contain, true);
    document.addEventListener("mouseup", select);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("mousemove", point);
      document.removeEventListener("click", contain, true);
      document.removeEventListener("mouseup", select);
      document.removeEventListener("keydown", escape);
    };
  }, [mode, threads, busy, active]);
  async function send(toAgent: boolean) {
    if (!thread || !thread.draft.trim() || busy) return;
    const body = thread.draft.trim();
    const key = `${thread.id}:${toAgent ? agent : "comment"}:${body}`;
    if (submission.current?.pending || (submission.current?.key === key && submission.current.done))
      return;
    const author = "You";
    const turn: Turn = { id: crypto.randomUUID(), author, body };
    if (!toAgent) {
      submission.current = {
        key,
        operationId: crypto.randomUUID(),
        message: "",
        pending: false,
        done: true,
      };
      setThreads((items) =>
        items.map((item) =>
          item.id === thread.id ? { ...item, turns: [...item.turns, turn], draft: "" } : item,
        ),
      );
      return;
    }
    if (!agent) {
      setError("Choose an agent in Connection first.");
      return;
    }
    const attempt =
      submission.current?.key === key
        ? submission.current
        : { key, operationId: crypto.randomUUID(), message: "", pending: false, done: false };
    submission.current = attempt;
    submission.current.pending = true;
    setBusy(true);
    setError("");
    try {
      const message = `[Handbook embed discussion]\nPage: ${location.href}\nAnchor: ${thread.anchor.path}\nQuote: ${thread.anchor.quote || "Component annotation"}\nConversation:\n${thread.turns
        .slice(-8)
        .map((item) => `${item.author}: ${item.body}`)
        .join("\n")}\nYou: ${body}`;
      submission.current.message ||= message;
      const result = await api("send", {
        agent,
        message: attempt.message,
        operationId: attempt.operationId,
      });
      if (typeof result.requestId !== "string")
        throw new Error("No request receipt returned. Check TMT before sending again.");
      turn.request = result.requestId;
      turn.recipient = agents.find((item) => item.id === agent)?.name ?? "Agent";
      turn.state = "◆ Waiting for reply";
      submission.current.done = true;
      setThreads((items) =>
        items.map((item) =>
          item.id === thread.id
            ? {
                ...item,
                turns: item.turns.some((existing) => existing.request === turn.request)
                  ? item.turns
                  : [...item.turns, turn],
                draft: "",
              }
            : item,
        ),
      );
    } catch (fault) {
      setError(fault instanceof Error ? fault.message : String(fault));
    } finally {
      if (submission.current) submission.current.pending = false;
      setBusy(false);
    }
  }
  const checkReply = useCallback(async (threadId: string, turn: Turn) => {
    if (!turn.request) return;
    const result = await api(`result/${turn.request}`);
    const response = result.response as { message?: string } | string | undefined;
    const body = typeof response === "string" ? response : response?.message;
    if (body === undefined) return;
    setThreads((items) =>
      items.map((item) => {
        if (
          item.id !== threadId ||
          item.turns.some((entry) => entry.id === turn.id && entry.state === "● Replied")
        )
          return item;
        return {
          ...item,
          turns: [
            ...item.turns.map((entry) =>
              entry.id === turn.id ? { ...entry, state: "● Replied" } : entry,
            ),
            { id: `reply-${turn.request}`, author: turn.recipient ?? "Agent", body },
          ],
        };
      }),
    );
  }, []);
  useEffect(() => {
    if (!mode || !thread) return;
    const pending = thread.turns.filter((turn) => turn.request && turn.state !== "● Replied");
    if (!pending.length) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const refresh = async () => {
      for (const turn of pending) {
        if (cancelled) return;
        try {
          await checkReply(thread.id, turn);
        } catch (fault) {
          if (!cancelled) setError(fault instanceof Error ? fault.message : String(fault));
          return;
        }
      }
      if (!cancelled) timer = setTimeout(refresh, 3000);
    };
    void refresh();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [mode, thread, checkReply]);
  const position = thread && positions[thread.id];
  const panelHeight = Math.min(
    Math.max(420, window.innerHeight * 0.72),
    720,
    window.innerHeight - 112,
  );
  const panelTop = position
    ? Math.max(
        16,
        Math.min(window.innerHeight - panelHeight - 88, position.top - window.scrollY + 32),
      )
    : 16;
  const panelLeft = position
    ? Math.max(
        8,
        Math.min(
          window.innerWidth - Math.min(400, window.innerWidth - 16) - 8,
          position.left - 350,
        ),
      )
    : Math.max(8, window.innerWidth - 416);
  return (
    <div
      onKeyDown={(event) => {
        if (!isImeConfirmation(event.nativeEvent, false) && event.repeat && event.key === "Enter")
          event.preventDefault();
      }}
    >
      <div className="ce-controls" role="group" aria-label="Colab controls">
        <button
          ref={fab}
          className={`ce-icon-action ${mode ? "ce-on" : ""}`}
          aria-label="Colab mode"
          aria-pressed={mode}
          aria-keyshortcuts="c"
          onClick={toggleMode}
        >
          <MousePointer2 size={18} strokeWidth={1.75} />
          <span className="ce-tooltip">Colab mode · C</span>
        </button>
        <button
          className={`ce-icon-action ${setup ? "ce-on" : ""}`}
          aria-label="Colab details"
          aria-expanded={setup}
          onClick={() => {
            setSetup(!setup);
            setActive(null);
          }}
        >
          <PanelRightOpen size={18} strokeWidth={1.75} />
          <span className="ce-tooltip">Connection & threads</span>
        </button>
      </div>
      {mode && hover && <div className="ce-hover" style={{ position: "fixed", ...hover }} />}
      {mode && !active && (
        <div className="ce-instructions">Hover & click a component · Select text to annotate</div>
      )}
      {mode &&
        threads.map(
          (item, index) =>
            !item.resolved &&
            positions[item.id] && (
              <button
                key={item.id}
                className={`ce-marker ${item.resolved ? "ce-resolved" : ""}`}
                style={{ position: "absolute", ...positions[item.id] }}
                aria-label={`Open annotation ${index + 1}: ${item.anchor.quote.slice(0, 60) || "Component"}`}
                onClick={() => {
                  setActive(item.id);
                  setSetup(false);
                }}
              >
                {item.resolved ? "✓" : "◇"}
                {index + 1}
              </button>
            ),
        )}
      {setup && (
        <section className="ce-panel ce-setup" role="dialog" aria-label="Colab connection">
          <header>
            <strong>◇ Colab · Connection</strong>
            <button onClick={() => setSetup(false)}>Close</button>
          </header>
          <p role="status">{status}</p>
          <p className="ce-muted">
            This preview uses local TMT access. Remote device pairing is separate.
          </p>
          <button onClick={() => void connect()}>Reconnect</button>
          {remote ? (
            <a href={remote} target="_blank" rel="noreferrer">
              Open Remote / pair a device ↗
            </a>
          ) : (
            <p>
              Start Remote in your terminal: <code>tmt remote serve</code>
            </p>
          )}
          <label>Discussion agent</label>
          <button className="ce-field" aria-expanded={choice} onClick={() => setChoice(!choice)}>
            {agents.find((item) => item.id === agent)?.name ?? "Choose agent"} ▾
          </button>
          {choice && (
            <div className="ce-agent-list">
              {agents.map((item) => (
                <button
                  key={item.id}
                  aria-pressed={item.id === agent}
                  onClick={() => {
                    setAgent(item.id);
                    setChoice(false);
                  }}
                >
                  {item.name} · {item.presence}
                </button>
              ))}
            </div>
          )}
          <h3>Annotations</h3>
          {threads.length === 0 && (
            <p className="ce-muted">Select something on the page to start.</p>
          )}
          {threads.map((item, index) => (
            <button
              key={item.id}
              className="ce-thread-link"
              onClick={() => {
                setMode(true);
                setSetup(false);
                setActive(item.id);
                resolve(item.anchor)?.scrollIntoView({ block: "center", behavior: "smooth" });
              }}
            >
              {index + 1}. {item.anchor.quote.slice(0, 65) || "Component"}
              <small>
                {!resolve(item.anchor)
                  ? "✗ Anchor changed · reattach"
                  : item.resolved
                    ? "○ Resolved"
                    : "◆ Open"}
                {item.draft ? " · Draft kept" : ""}
              </small>
            </button>
          ))}
        </section>
      )}
      {mode && thread && (
        <section
          className="ce-panel ce-discussion"
          style={{
            top: panelTop,
            left: panelLeft,
            height: panelHeight,
            maxHeight: window.innerHeight - 112,
          }}
          role="dialog"
          aria-label="Annotation discussion"
        >
          <header>
            <strong>◇ Colab · Annotation</strong>
            <div className="ce-header-actions">
              <button
                className="ce-icon-action"
                aria-label={thread.resolved ? "Reopen annotation" : "Resolve annotation"}
                disabled={busy}
                onClick={() => {
                  update(thread.id, { resolved: !thread.resolved });
                  if (!thread.resolved) setActive(null);
                }}
              >
                {thread.resolved ? (
                  <RotateCcw size={18} strokeWidth={1.75} aria-hidden="true" />
                ) : (
                  <Check size={18} strokeWidth={1.75} aria-hidden="true" />
                )}
                <span className="ce-tooltip" role="tooltip">
                  {thread.resolved ? "Reopen" : "Resolve"}
                </span>
              </button>
              <button
                className="ce-icon-action"
                aria-label="Close annotation"
                disabled={busy}
                onClick={() => {
                  setActive(null);
                  fab.current?.focus();
                }}
              >
                <X size={18} strokeWidth={1.75} aria-hidden="true" />
                <span className="ce-tooltip" role="tooltip">
                  Close
                </span>
              </button>
            </div>
          </header>
          <div className="ce-messages" ref={messages}>
            <blockquote>{thread.anchor.quote || "Component annotation"}</blockquote>
            {!position && (
              <p className="ce-warning">
                ✗ Anchor changed. Select the content again to create a new annotation; this
                discussion is kept.
              </p>
            )}
            <div className="ce-turns">
              {thread.turns.map((turn) => (
                <article key={turn.id}>
                  <strong>{turn.author}</strong>
                  <p>{turn.body}</p>
                  {turn.state && (
                    <small>
                      {turn.state}{" "}
                      {turn.request && turn.state !== "● Replied" && (
                        <button
                          disabled={busy}
                          onClick={() =>
                            void checkReply(thread.id, turn).catch((fault: Error) =>
                              setError(fault.message),
                            )
                          }
                        >
                          Check reply
                        </button>
                      )}
                    </small>
                  )}
                </article>
              ))}
            </div>
          </div>
          <div className="ce-composer">
            <label htmlFor="ce-draft">Reply{thread.draft ? " · Draft kept" : ""}</label>
            <textarea
              id="ce-draft"
              ref={composer}
              value={thread.draft}
              disabled={busy}
              placeholder="Discuss this part…"
              onChange={(event) => {
                if (!submission.current?.pending) submission.current = null;
                update(thread.id, { draft: event.target.value });
              }}
              onCompositionStart={(event) => {
                event.currentTarget.dataset.composing = "true";
              }}
              onCompositionEnd={(event) => {
                event.currentTarget.dataset.composing = "false";
              }}
              onKeyDown={(event) => {
                if (
                  isImeConfirmation(
                    event.nativeEvent,
                    event.currentTarget.dataset.composing === "true",
                  )
                )
                  return;
                if (event.key === "Enter" && !event.shiftKey) {
                  event.preventDefault();
                  if (!event.repeat) void send(true);
                }
              }}
            />
            <div className="ce-actions">
              <button disabled={busy || !thread.draft.trim()} onClick={() => void send(false)}>
                Comment
              </button>
              <button
                className="ce-primary"
                disabled={busy || !thread.draft.trim() || !agent}
                onClick={() => void send(true)}
              >
                {busy
                  ? "◐ Sending"
                  : `Ask ${agents.find((item) => item.id === agent)?.name ?? "agent"}`}
              </button>
            </div>
          </div>
        </section>
      )}
      {mode && error && (
        <div className="ce-error" role="alert">
          ✗ {error}
          <button onClick={() => setError("")}>Dismiss</button>
        </div>
      )}
    </div>
  );
}
let root: ReturnType<typeof createRoot> | undefined = import.meta.hot?.data.root;
export function mountColabEmbed() {
  if (root) return;
  const host = document.createElement("div");
  host.id = "tmt-colab-embed";
  document.body.append(host);
  root = createRoot(host);
  root.render(<ColabEmbed />);
}
if (import.meta.hot) {
  import.meta.hot.dispose((data) => {
    data.root = root;
  });
  import.meta.hot.accept((module) => {
    if (module) root?.render(<module.ColabEmbed />);
  });
}
