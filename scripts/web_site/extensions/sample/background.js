// The background of the gate's sample extension (Kaleidoscope N7b): storage, a
// registered content script, a dynamic blocking rule, and answers to its content script.
chrome.runtime.onInstalled.addListener(d => {
  chrome.storage.local.get({ installs: 0 }).then(v => chrome.storage.local.set({ installs: v.installs + 1 }));
  console.log('background installed ' + d.reason);
});
chrome.runtime.onStartup.addListener(() => console.log('background started'));
chrome.scripting.registerContentScripts([{ id: 'late', matches: ['*://*/*'], js: ['registered.js'], runAt: 'document_idle' }])
  .then(() => console.log('content script registered'), e => console.log('register failed ' + e.message));
chrome.declarativeNetRequest.updateDynamicRules({ removeRuleIds: [9], addRules: [{ id: 9, priority: 1, action: { type: 'block' }, condition: { urlFilter: 'blocked-dynamic.png' } }] })
  .then(() => console.log('dynamic rule added'));
chrome.runtime.onMessage.addListener((m, sender, reply) => {
  if (!m || !m.hello) return;
  chrome.storage.local.get('installs').then(v => reply({ ok: true, host: m.hello, installs: v.installs, tab: sender.tab && sender.tab.url }));
  return true;
});
