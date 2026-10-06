import type { ReactNode } from "react";
import { BoardStepScene } from "./journey-scenes";
import { ColabPage } from "../chapter-scenes/ColabPage";
import { Inline } from "../components/Inline";
import { LocalLink } from "../components/LocalLink";
import { Tag } from "../components/marks";
import { pages } from "../chapters";
import { useStrings } from "../lang/useStrings";

type Key = "squad" | "colab";

// Where each tour section links, and the scene it borrows from that chapter.
const TOUR: { key: Key; path: string; scene: () => ReactNode }[] = [
  { key: "squad", path: "/extensions/squad", scene: () => <BoardStepScene /> },
  { key: "colab", path: "/extensions/colab", scene: () => <ColabPage /> },
];

// One edge-to-edge band of the home page: an eyebrow, a large claim, one
// sentence, then the scene. Bands are separated by a full-width rule.
export function Band({
  id,
  eyebrow,
  title,
  status,
  children,
}: {
  id: string;
  eyebrow: string;
  title: string;
  status?: (typeof pages)[number]["status"];
  children: ReactNode;
}) {
  return (
    <section id={id} className="home-band">
      <div className="home-eyebrow">{eyebrow}</div>
      <h2 className="home-band-title">
        <Inline text={title} />
        {status && <Tag kind={status.kind}>{status.label}</Tag>}
      </h2>
      {children}
    </section>
  );
}

// The home page's tour of the layers that sit on the core, each with the
// chapter's own scene, so the home page and the chapters show the same thing.
export function Showcase() {
  const { showcase } = useStrings().home;
  return (
    <>
      {TOUR.map(({ key, path, scene }) => {
        const section = showcase.sections[key];
        const status = pages.find((page) => page.path === path)?.status;
        const shown = status && status.kind !== "built in" && status.kind !== "alpha";
        return (
          <Band
            key={key}
            id={`tour-${key}`}
            eyebrow={section.eyebrow}
            title={section.title}
            status={shown ? status : undefined}
          >
            <p className="mb-6 max-w-[62ch] text-[17px] leading-normal sm:text-[18px]">
              <Inline text={section.text} />
            </p>
            {scene()}
            <p className="mt-4 mb-0 font-mono text-sm">
              <LocalLink to={path} className="text-accent">
                {showcase.chapter}
              </LocalLink>
            </p>
          </Band>
        );
      })}
    </>
  );
}

// The top of a band whose heading and body are written in the chapter file
// itself (start, install): the full-width rule and the eyebrow, plus the
// heading when the strings carry one.
export function BandHead({ name }: { name: "start" | "install" }) {
  const section = useStrings().home.showcase.sections[name];
  return (
    <div className="home-band-head">
      <div className="home-eyebrow">{section.eyebrow}</div>
      {"title" in section && (
        <h2 id={`${name}-band`} className="home-band-title">
          <Inline text={section.title} />
        </h2>
      )}
    </div>
  );
}
