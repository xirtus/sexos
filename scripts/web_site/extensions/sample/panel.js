// The sample extension's side panel page (Kaleidoscope N13e): it lays out to the
// panel's width and has the extension API, chrome.sidePanel included. Its timer keeps
// running while the page beside it has the keyboard.
console.log('panel ready ' + innerWidth + ' ' + typeof chrome.sidePanel.open + ' ' + location.href);
let ticks = 0;
const shown = setInterval(() => {
  ticks++;
  document.querySelector('p').textContent = 'Ticks: ' + ticks;
  console.log('panel tick ' + ticks);
  if (ticks >= 3) clearInterval(shown);
}, 300);
