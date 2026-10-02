import type { ReactNode } from "react";

// A terminal window: dots, a title and the scene's body. Always the dark
// terminal look, like the other mockups.
export function Window({
  title,
  children,
  footer,
  label,
}: {
  title: string;
  children: ReactNode;
  footer?: ReactNode;
  label?: string;
}) {
  return (
    <div
      role={label ? "img" : undefined}
      aria-label={label}
      className="w-full overflow-hidden rounded-lg border border-term-edge bg-term text-t-text shadow-[0_30px_60px_-30px_#000]"
    >
      <div className="flex items-center gap-1.5 bg-term-bar px-2.5 py-2 font-mono text-xs text-t-dim">
        <i className="size-2.5 rounded-full bg-term-edge" />
        <i className="size-2.5 rounded-full bg-term-edge" />
        <i className="size-2.5 rounded-full bg-term-edge" />
        <span className="ml-2">{title}</span>
      </div>
      {children}
      {footer}
    </div>
  );
}

// The tmux status line at the foot of a window; `message` is the one
// attention-colored slot, shown only while something waits on you.
export function TmuxBar({ window: windowName, message }: { window: string; message?: ReactNode }) {
  return (
    <div className="flex bg-t-accent font-mono text-xs leading-none font-semibold text-term-bar">
      <span className="bg-term-bar px-2 py-1.5 text-t-accent">[tmt]</span>
      <span className="px-2 py-1.5">{windowName}</span>
      {message && <span className="ml-auto bg-t-waiting px-2 py-1.5">{message}</span>}
    </div>
  );
}
