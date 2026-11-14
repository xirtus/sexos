document.body.setAttribute('data-recoloured','yes');console.log('extension content script ran');
chrome.runtime.sendMessage({ hello: location.host }).then(r => console.log('extension reply ' + JSON.stringify(r)));
