import { Inline } from "../components/Inline";
import { LocalLink } from "../components/LocalLink";
import { Cmd } from "../components/Cmd";
import { MessageTravel } from "../chapter-scenes/MessageTravel";
import { useStrings } from "../lang/useStrings";
import { Showcase } from "./Showcase";

export function Hero() {
  const { home } = useStrings();
  return (
    <div className="home-hero">
      <div className="home-eyebrow">{home.eyebrow}</div>
      <h1 className="home-title">
        tmt<span className="text-dim">.</span>
      </h1>
      <p className="home-lede">{home.tagline}</p>
      <div className="home-install">
        <div className="home-eyebrow">{home.install}</div>
        <Cmd>{`$ curl -fsSL https://github.com/pj-tmt/tmt/releases/latest/download/install.sh | sh`}</Cmd>
      </div>
      <p className="home-actions">
        <LocalLink to="/" hash="install" className="home-primary">
          {home.install} →
        </LocalLink>
        <LocalLink to="/" hash="start" className="home-secondary">
          {home.start}
        </LocalLink>
      </p>
    </div>
  );
}

export function HomePage() {
  const { home } = useStrings();
  const working = home.showcase.sections.working;
  return (
    <>
      <Hero />
      <section id="start" className="home-band">
        <div className="home-eyebrow">01 / {working.eyebrow}</div>
        <h2 className="home-band-title">
          <Inline text={working.title} />
        </h2>
        <MessageTravel />
        <LocalLink to="/working">{home.showcase.chapter}</LocalLink>
      </section>
      <Showcase />
    </>
  );
}
