/** The leaf's chart entry: the painters, their pure models and the numbers behind them. */
export * from './chart-math.js';
export { createMatrix } from './chart-matrix.js';
export { matrixModel, type MatrixOptions, type MatrixParticipant } from './chart-matrix-model.js';
export { createSeriesChart } from './chart-series.js';
export { seriesModel, type SeriesChartOptions, type SeriesModel } from './chart-series-model.js';
export { createStrips, stripsLegend } from './chart-strips.js';
export {
  STRIP_STEPS,
  stripsModel,
  type StripRow,
  type StripsOptions,
} from './chart-strips-model.js';
