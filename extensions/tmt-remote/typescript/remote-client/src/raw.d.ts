/** Vite's `?raw` import: the file's text, inlined at build time. */
declare module '*?raw' {
  const text: string;
  export default text;
}

declare module '*remote-firebase-v1.js' {
  export const initialize: import('./firebase-auth.js').FirebaseClientModule['initialize'];
}

declare module '*remote-v1.js' {
  export const firebaseRemote: typeof import('./browser.js').firebaseRemote;
}
