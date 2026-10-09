/** Runtime import remains the one door-served SDK, never a second channel owner. */
declare module 'remote-browser-sdk' {
  export const budget: typeof import('./browser.js').budget;
  export const management: typeof import('./browser.js').management;
  export const reopenSession: typeof import('./browser.js').reopenSession;
  export const ClientError: typeof import('./browser.js').ClientError;
  export const RefusalError: typeof import('./browser.js').RefusalError;
}
