import { localTransport } from './transport.js';
export const previewTransport = localTransport('My space', [
  {
    id: 'welcome',
    title: 'A shared page',
    sharing: 'private',
    source: `<!doctype html><html><head><title>A shared page</title><style>body{font:17px/1.6 system-ui;max-width:640px;margin:48px auto;padding:0 24px;background:white;color:#243047}h1{font-size:34px;letter-spacing:-.03em}button{font:inherit;padding:8px 14px;border:1px solid;border-radius:6px;background:transparent;cursor:pointer}</style></head><body><p>HELLO, COLAB</p><h1>Small pages, shared ideas.</h1><p>This is your page canvas. HTML and JavaScript run here, inside an isolated frame. The controls around it belong to Colab.</p><button id="counter">Try the page: 0</button><script>let n=0;document.getElementById('counter').onclick=()=>{document.getElementById('counter').textContent='Try the page: '+(++n)}</script></body></html>`,
  },
  {
    id: 'notes',
    title: 'Working notes',
    sharing: 'private',
    source:
      '<h1>Working notes</h1><p>Use a page to explain an idea, sketch an interface or share a small experiment.</p>',
  },
]);
