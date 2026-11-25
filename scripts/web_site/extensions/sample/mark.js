// Runs in the extension's isolated world: the page's globals are not here, and the
// page never sees this script's. The document is the same one.
window.extSecret = 1;
console.log('content sees ' + typeof pageSecret + ' ' + typeof window.pageSecret);
new MutationObserver(rs => { for (const r of rs) for (const n of r.addedNodes) if (n.id) console.log('content saw ' + n.id); }).observe(document.body, { childList: true });
document.body.setAttribute('data-recoloured','yes');console.log('extension content script ran');
chrome.runtime.sendMessage({ hello: location.host }).then(r => console.log('extension reply ' + JSON.stringify(r)));
if (location.href.indexOf('probe=1') >= 0) chrome.runtime.sendMessage({ armScriptProbe: true });
