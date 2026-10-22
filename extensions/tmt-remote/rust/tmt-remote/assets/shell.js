// The one TMT web app (shell.html) starts from the same served SDK. The import is
// dynamic so the SDK never holds back the page's DOMContentLoaded (the pairing form stays usable).
void import('/sdk/remote-v1.js').then(({ shellApp }) => shellApp());
