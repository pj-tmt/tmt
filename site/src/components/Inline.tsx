import type { ReactNode } from "react";

// Renders a site string with `code` and *emphasis*. Nothing else is markup, so
// a translated string cannot inject HTML.
export function Inline({ text }: { text: string }): ReactNode {
  return text.split(/(`[^`]+`|\*[^*]+\*)/).map((part, index) => {
    if (part.startsWith("`") && part.endsWith("`") && part.length > 2)
      return (
        <code key={index} className="bg-accent-soft px-[.35em] py-[.12em] font-mono text-[.84em]">
          {part.slice(1, -1)}
        </code>
      );
    if (part.startsWith("*") && part.endsWith("*") && part.length > 2)
      return (
        <em key={index} className="whitespace-nowrap text-accent not-italic">
          {part.slice(1, -1)}
        </em>
      );
    return part;
  });
}
