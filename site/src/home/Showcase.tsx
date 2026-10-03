import type { ReactNode } from "react";
import { MarkLegend } from "../chapter-scenes/MarkLegend";
import { ColabPage } from "../chapter-scenes/ColabPage";
import { MeetRoom } from "../chapter-scenes/MeetRoom";
import { MessageTravel } from "../chapter-scenes/MessageTravel";
import { Inline } from "../components/Inline";
import { LocalLink } from "../components/LocalLink";
import { Tag } from "../components/marks";
import { pages } from "../chapters";
import { useStrings } from "../lang/useStrings";

type Key = "working" | "squad" | "colab" | "meet";

// Where each tour section links, and the scene it borrows from that chapter.
const TOUR: { key: Key; path: string; scene: () => ReactNode }[] = [
  { key: "working", path: "/working", scene: () => <MessageTravel /> },
  { key: "squad", path: "/extensions/squad", scene: () => <MarkLegend /> },
  { key: "colab", path: "/extensions/colab", scene: () => <ColabPage /> },
  { key: "meet", path: "/extensions/meet", scene: () => <MeetRoom /> },
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
    <section id={id} className="tour-band border-t-2 border-text py-12 sm:py-16">
      <div className="font-mono text-xs leading-none font-semibold tracking-[0.06em] text-accent before:text-dim before:content-['##_']">
        {eyebrow}
      </div>
      <h2 className="mt-2.5 mb-4 font-mono text-[clamp(26px,3.6vw,40px)] leading-[1.05] font-bold tracking-[-0.02em] text-balance">
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
            <div className="mt-8 grid grid-cols-1 items-start gap-8 lg:grid-cols-[minmax(0,2fr)_minmax(0,3fr)] lg:gap-12">
              <div>
                <p className="mb-6 max-w-[62ch] text-[17px] leading-normal sm:text-[18px]">
                  <Inline text={section.text} />
                </p>
                <p className="mt-4 mb-0 font-mono text-sm">
                  <LocalLink to={path} className="text-accent">
                    {showcase.chapter}
                  </LocalLink>
                </p>
              </div>
              <div className="min-w-0">{scene()}</div>
            </div>
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
    <div className="band-head mt-0 border-t-2 border-text pt-12 sm:pt-16">
      <div className="font-mono text-xs leading-none font-semibold tracking-[0.06em] text-accent before:text-dim before:content-['##_']">
        {section.eyebrow}
      </div>
      {"title" in section && (
        <h2
          id={`${name}-band`}
          className="mt-2.5 mb-4 font-mono text-[clamp(26px,3.6vw,40px)] leading-[1.05] font-bold tracking-[-0.02em] text-balance"
        >
          <Inline text={section.title} />
        </h2>
      )}
    </div>
  );
}
