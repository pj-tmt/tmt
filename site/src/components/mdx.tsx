import { Link } from "@tanstack/react-router";
import type { MDXComponents } from "mdx/types";
import type { ComponentProps, ReactNode } from "react";

export function slug(text: string) {
  return text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (node && typeof node === "object" && "props" in node)
    return textOf((node.props as { children?: ReactNode }).children);
  return "";
}

// Internal links ("/drivers", "/extensions/squad#…") go through the router, so
// they respect the Pages base path; anything else is an ordinary link.
export function A({ href = "", children, ...rest }: ComponentProps<"a">) {
  if (href.startsWith("/")) {
    const [to, hash] = href.split("#");
    return (
      <Link to={to} hash={hash} className="text-accent underline underline-offset-[3px]">
        {children}
      </Link>
    );
  }
  return (
    <a href={href} {...rest}>
      {children}
    </a>
  );
}

export const components: MDXComponents = {
  h3: ({ children }) => (
    <h3
      id={slug(textOf(children))}
      className="mt-10 mb-2.5 scroll-mt-6 font-display text-[17px] leading-tight font-semibold text-balance"
    >
      {children}
    </h3>
  ),
  h4: ({ children }) => (
    <h4
      id={slug(textOf(children))}
      className="mt-7 mb-2 scroll-mt-6 font-display text-[15px] leading-tight font-semibold"
    >
      {children}
    </h4>
  ),
  p: (props) => <p className="my-0 mb-3.5" {...props} />,
  ul: (props) => <ul className="mb-3.5 pl-[1.1em] [&>li]:mb-1.5 [&>li]:list-disc" {...props} />,
  ol: (props) => <ol className="mb-3.5 pl-[1.3em] [&>li]:mb-1.5 [&>li]:list-decimal" {...props} />,
  a: A,
  code: (props) => (
    <code
      className="rounded-[3px] bg-accent-soft px-[.35em] py-[.12em] font-mono text-[.84em]"
      {...props}
    />
  ),
  table: (props) => (
    <div className="term-scroll my-4 w-full">
      <table {...props} />
    </div>
  ),
  blockquote: (props) => (
    <blockquote
      className="my-4 border-l-[3px] border-accent py-1 pl-4 text-base text-muted"
      {...props}
    />
  ),
};
