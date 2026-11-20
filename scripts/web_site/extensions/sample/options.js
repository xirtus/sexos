// The sample extension's options page (Kaleidoscope N7c): an extension page in a tab has
// the whole API, and can ask its background and read its storage.
chrome.runtime.sendMessage({ hello: 'options' }).then(r => {
  console.log('options reply ' + JSON.stringify(r));
  document.getElementById('out').textContent = 'Installed ' + r.installs + ' time(s)';
});
chrome.storage.local.get('installs').then(v => console.log('options storage ' + v.installs + ' ' + location.href));
