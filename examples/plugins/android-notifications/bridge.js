// Minimal Android setup bridge. Google handles password entry and challenges.
// Never forward passwords, page contents, arbitrary URLs, or console output.
(() => {
  if (location.origin !== 'https://accounts.google.com' || window.top !== window) return;
  const send = (event, value) => window.webkit.messageHandlers.androidSetup.postMessage(JSON.stringify({event, value}));
  window.mm = {
    showView: () => send('show'),
    closeView: () => send('close'),
    setAccountIdentifier: value => send('account', value),
    log: () => {},
    // Optional native capabilities (DroidGuard, passkeys, addAccount, device
    // data) must be absent, not no-op stubs: Google otherwise waits for their
    // asynchronous result callbacks. This is the minimal desktop setup bridge.
  };
})();
