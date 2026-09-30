import { useEffect, useState, type RefObject } from "react";

export type Sleep = (ms: number) => Promise<boolean>;

type Script<F> = {
  count: number;
  // The finished frame of step k, shown without animation.
  still: (k: number) => F;
  // Plays step k (the previous step's frame is on screen). Returns false once
  // cancelled, when a newer step took over.
  animate: (k: number, emit: (frame: F) => void, sleep: Sleep) => Promise<boolean>;
  hold: (k: number) => number;
};

const reducedMotion = () => {
  try {
    return matchMedia("(prefers-reduced-motion: reduce)").matches;
  } catch {
    return false;
  }
};

// A step-by-step walkthrough that starts when it scrolls into view, loops,
// and stops on any manual step. Reduced motion shows finished steps only.
// The controller lives outside render: React state holds only what is drawn.
class Controller<F> {
  token = 0;
  playing: boolean;
  script: Script<F>;
  constructor(
    script: Script<F>,
    playing: boolean,
    private readonly draw: (frame: F) => void,
    private readonly step: (index: number) => void,
    private readonly play: (playing: boolean) => void,
  ) {
    this.script = script;
    this.playing = playing;
  }
  index = 0;
  use(script: Script<F>) {
    this.script = script;
  }
  async show(k: number, animate: boolean): Promise<void> {
    const mine = ++this.token;
    const sleep: Sleep = (ms) =>
      new Promise((resolve) => setTimeout(() => resolve(mine === this.token), ms));
    this.index = k;
    this.step(k);
    if (animate && !reducedMotion()) {
      const finished = await this.script.animate(
        k,
        (frame) => mine === this.token && this.draw(frame),
        sleep,
      );
      if (!finished) return;
    }
    this.draw(this.script.still(k));
    if (animate && (await sleep(this.script.hold(k))) && this.playing)
      return this.show((k + 1) % this.script.count, true);
  }
  start() {
    const mine = this.token;
    setTimeout(() => {
      if (mine === this.token && this.playing) void this.show(1 % this.script.count, true);
    }, this.script.hold(0));
  }
  stop() {
    this.token++;
  }
  setPlaying(playing: boolean) {
    this.playing = playing;
    this.play(playing);
  }
  prev() {
    this.setPlaying(false);
    void this.show((this.index - 1 + this.script.count) % this.script.count, false);
  }
  next() {
    this.setPlaying(false);
    void this.show((this.index + 1) % this.script.count, true);
  }
  toggle() {
    this.setPlaying(!this.playing);
    if (this.playing) void this.show((this.index + 1) % this.script.count, true);
    else this.stop();
  }
}

export function useStepPlayer<F>(script: Script<F>, root: RefObject<HTMLDivElement | null>) {
  const [frame, setFrame] = useState<F>(() => script.still(0));
  const [index, setIndex] = useState(0);
  const [playing, setPlaying] = useState(() => !reducedMotion());
  const [controller] = useState(
    () => new Controller(script, playing, setFrame, setIndex, setPlaying),
  );

  useEffect(() => controller.use(script));

  useEffect(() => {
    const element = root.current;
    if (!element || !controller.playing) return;
    let started = false;
    const start = () => {
      if (started) return;
      started = true;
      controller.start();
    };
    if (!("IntersectionObserver" in window)) {
      start();
      return () => controller.stop();
    }
    const observer = new IntersectionObserver(
      (entries) => entries.some((entry) => entry.isIntersecting) && start(),
      {
        threshold: 0.4,
      },
    );
    observer.observe(element);
    return () => {
      observer.disconnect();
      controller.stop();
    };
  }, [controller, root]);

  return {
    frame,
    index,
    playing,
    prev: () => controller.prev(),
    next: () => controller.next(),
    toggle: () => controller.toggle(),
  };
}

export function DemoControls({
  caption,
  index,
  count,
  playing,
  prev,
  next,
  toggle,
}: {
  caption: string;
  index: number;
  count: number;
  playing: boolean;
  prev: () => void;
  next: () => void;
  toggle: () => void;
}) {
  const button =
    "cursor-pointer rounded-md border border-rule bg-sheet px-3 py-2 font-mono text-[13px] leading-none text-text hover:border-accent";
  return (
    <>
      <div className="mt-2 flex gap-1.5" aria-hidden="true">
        {Array.from({ length: count }, (_, k) => (
          <i
            key={k}
            className={`h-[3px] w-[18px] rounded-sm ${k <= index ? "bg-accent" : "bg-rule"}`}
          />
        ))}
      </div>
      <div className="mt-2.5 flex items-center gap-2">
        <p className="m-0 min-h-[2.9em] flex-1 text-[15.5px]" aria-live="polite">
          {caption}
        </p>
        <button type="button" className={button} onClick={prev} aria-label="Previous step">
          ◀
        </button>
        <button
          type="button"
          className={button}
          onClick={toggle}
          aria-label={playing ? "Pause" : "Play"}
        >
          {playing ? "❚❚" : "▶"}
        </button>
        <button type="button" className={button} onClick={next} aria-label="Next step">
          ▶▶
        </button>
      </div>
    </>
  );
}
