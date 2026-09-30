import type { ReactNode } from "react";

// A terminal line in a demo: plain text, or pieces with a tone.
export type Tone = "dm" | "wt" | "pr" | "ok" | "er" | "hd" | "sel";
export type Seg = string | { t: string; k: Tone };
export type Line = Seg | Seg[];

export const TONE: Record<Tone, string> = {
  dm: "text-t-dim",
  wt: "text-t-waiting",
  pr: "text-t-accent",
  ok: "text-t-working",
  er: "text-t-blocked",
  hd: "text-t-accent font-bold",
  sel: "bg-t-selection text-t-text",
};

export const dm = (t: string): Seg => ({ t, k: "dm" });
export const wt = (t: string): Seg => ({ t, k: "wt" });
export const ok = (t: string): Seg => ({ t, k: "ok" });
export const sh = (t: string): Seg[] => [{ t: "$ ", k: "pr" }, t];

export function seg(piece: Seg, key?: number): ReactNode {
  return typeof piece === "string" ? (
    piece
  ) : (
    <span key={key} className={TONE[piece.k]}>
      {piece.t}
    </span>
  );
}

export function line(value: Line): ReactNode {
  return Array.isArray(value) ? value.map((piece, index) => seg(piece, index)) : seg(value);
}

export const pad = (text: string, width: number) => text.padEnd(width).slice(0, width);
