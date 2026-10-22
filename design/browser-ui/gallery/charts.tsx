import { useLayoutEffect, useRef } from 'react';
import { createMatrix } from '../src/chart-matrix.js';
import { topLayers } from '../src/chart-math.js';
import { createSeriesChart } from '../src/chart-series.js';
import { createStrips } from '../src/chart-strips.js';
import { Pair, Section } from './kit.js';
import {
  burn,
  burnAverage,
  hours,
  manyPeople,
  people,
  seatLayers,
  sentBetween,
  slotRange,
  strips,
  weeklyLeft,
  weeklyPace,
  weeklyX,
} from './chart-sample.js';

/** Mounts one vanilla chart element; the gallery shows it exactly as a page would build it. */
function Mount({ build }: { build: () => HTMLElement }) {
  const host = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    host.current?.replaceChildren(build());
  }, [build]);
  return <div ref={host} />;
}

const stackBuild = (share: boolean) => () =>
  createSeriesChart({
    title: share ? 'Share of tokens by seat' : 'Tokens by seat',
    kind: share ? 'share' : 'stack',
    x: hours,
    series: topLayers(seatLayers(11), 7),
  });

export function ChartsSection() {
  return (
    <Section id="charts">
      <Pair
        title="Stacked and share"
        note="Eleven seats fold into seven plus Other. Arrow keys, Home and End read each hour."
      >
        {() => (
          <>
            <Mount build={stackBuild(false)} />
            <Mount build={stackBuild(true)} />
          </>
        )}
      </Pair>
      <Pair
        title="Weekly window and burn"
        note="A missing sample breaks the line; the dashed line is an even pace."
      >
        {() => (
          <>
            <Mount
              build={() =>
                createSeriesChart({
                  title: 'Claude, weekly percent left',
                  kind: 'line',
                  x: weeklyX,
                  series: [{ key: 'left', label: 'left', values: weeklyLeft }],
                  reference: { label: 'even pace', values: weeklyPace },
                  guides: [{ at: weeklyX.length - 1, label: 'reset' }],
                  max: 100,
                  format: (v) => `${Math.round(v)}%`,
                })
              }
            />
            <Mount
              build={() =>
                createSeriesChart({
                  title: 'Burn, points per hour',
                  kind: 'bars',
                  x: hours,
                  series: [{ key: 'burn', label: 'burn', values: burn }],
                  reference: { label: '6 h average', values: burnAverage },
                  format: (v) => v.toFixed(1),
                })
              }
            />
          </>
        )}
      </Pair>
      <Pair
        title="Who talks to whom"
        note="Self is blank, · is observed none, — is unavailable (night has no read)."
      >
        {() => (
          <>
            <Mount
              build={() =>
                createMatrix({
                  title: 'Messages, last 24 hours',
                  participants: people,
                  value: sentBetween,
                  sum: true,
                })
              }
            />
            <Mount
              build={() =>
                createMatrix({
                  title: 'Many agents scroll inside the figure',
                  participants: manyPeople,
                  value: sentBetween,
                })
              }
            />
          </>
        )}
      </Pair>
      <Pair
        title="Activity by seat"
        note="Square slots, left-aligned, oldest on the left; a hollow slot is not observed."
      >
        {() => (
          <Mount
            build={() =>
              createStrips({
                title: 'Tokens per hour, last 24 hours',
                rows: strips(6),
                slotCount: 24,
                slotRange,
                ends: ['24 h ago', 'now'],
              })
            }
          />
        )}
      </Pair>
      <Pair title="Empty states" note="No data is said plainly; nothing is drawn as zero.">
        {() => (
          <>
            <Mount
              build={() =>
                createSeriesChart({
                  title: 'Tokens by seat',
                  kind: 'stack',
                  x: hours,
                  series: [],
                  empty: 'No token observations yet.',
                })
              }
            />
            <Mount
              build={() => createMatrix({ title: 'Messages', participants: [], value: () => null })}
            />
            <Mount
              build={() =>
                createStrips({
                  title: 'Activity',
                  rows: [],
                  slotCount: 24,
                  slotRange,
                  ends: ['', ''],
                })
              }
            />
          </>
        )}
      </Pair>
    </Section>
  );
}
