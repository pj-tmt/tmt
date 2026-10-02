import { Link } from "@tanstack/react-router";
import type { ComponentProps } from "react";
import { withLang } from "../lang/languages";
import { useLang } from "../lang/useLang";

// A link to a handbook page that stays in the reader's language.
export function LocalLink({ to, ...rest }: ComponentProps<typeof Link> & { to: string }) {
  const { lang } = useLang();
  return <Link to={withLang(lang, to)} {...rest} />;
}
