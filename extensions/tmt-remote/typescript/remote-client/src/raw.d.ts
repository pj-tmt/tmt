/** Vite's `?raw` import: the file's text, inlined at build time. */
declare module '*?raw' {
  const text: string;
  export default text;
}
