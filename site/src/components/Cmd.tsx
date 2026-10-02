import { useState } from "react";

// A shell block. Lines starting with "$ " are commands a reader can copy; a
// trailing "  # …" is a dimmed comment; a line starting with "#" is a comment;
// anything else is output.
export function Cmd({ children }: { children: string }) {
  const lines = children.replace(/^\n+|\n+$/g, "").split("\n");
  const commands = lines
    .filter((line) => line.startsWith("$ "))
    .map((line) => stripComment(line.slice(2)).trimEnd());
  return (
    <div className="group relative my-4 w-full border-2 border-text bg-sheet text-text shadow-[5px_5px_0_var(--c-text)]">
      <pre className="term-scroll m-0 px-4 py-3.5 font-mono text-[13px] leading-[1.65] sm:text-sm">
        {lines.map((line, index) => (
          <div key={index}>{renderLine(line)}</div>
        ))}
      </pre>
      {commands.length > 0 && (
        <CopyButton text={commands.join("\n")} plural={commands.length > 1} />
      )}
    </div>
  );
}

function stripComment(text: string) {
  const at = text.search(/\s{2,}#\s/);
  return at < 0 ? text : text.slice(0, at);
}

function renderLine(line: string) {
  if (line === "") return " ";
  if (line.startsWith("#")) return <span className="text-muted">{line}</span>;
  if (!line.startsWith("$ ")) return <span className="text-muted">{line}</span>;
  const body = line.slice(2);
  const command = stripComment(body);
  const comment = body.slice(command.length);
  return (
    <>
      <span className="text-accent">$ </span>
      {command}
      {comment && <span className="text-muted">{comment}</span>}
    </>
  );
}

function CopyButton({ text, plural }: { text: string; plural: boolean }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    } catch {
      // Clipboard access can be refused; the text stays selectable.
    }
  };
  return (
    <button
      type="button"
      onClick={copy}
      aria-label={plural ? "Copy commands" : "Copy command"}
      className={`absolute top-1.5 right-1.5 cursor-pointer border bg-sheet px-2 py-1 font-mono text-[11px] leading-none opacity-100 transition-opacity sm:opacity-60 sm:group-hover:opacity-100 ${
        copied ? "border-working text-working" : "border-rule text-muted hover:text-text"
      }`}
    >
      {copied ? "copied" : "copy"}
    </button>
  );
}

// A config or code file: plain text with "# comments" dimmed, no copy button.
export function Code({ children }: { children: string }) {
  const lines = children.replace(/^\n+|\n+$/g, "").split("\n");
  return (
    <pre className="term-scroll my-4 w-full border-2 border-text bg-sheet px-4 py-3.5 font-mono text-[13px] leading-[1.65] text-text shadow-[5px_5px_0_var(--c-text)] sm:text-sm">
      {lines.map((line, index) => {
        const at = line.search(/(^|\s)#(\s|$)/);
        return (
          <div key={index}>
            {line === "" ? " " : at < 0 ? line : line.slice(0, at)}
            {at >= 0 && <span className="text-muted">{line.slice(at)}</span>}
          </div>
        );
      })}
    </pre>
  );
}
