// SPDX-License-Identifier: GPL-3.0-only
// `chrome.*` and `browser.*` for Kaleidoscope's extensions (slice N7b): what content
// scripts and background scripts call. Each extension gets one API object per realm
// (the page's realm for its content scripts, its own realm for its background).
// Messages between realms go through the browser: `post` hands one over, and the
// browser dispatches answers and events here as `ext\tJSON`.
//
// Only code the browser wrote can make an API object: it must give the key the browser
// handed this realm once, before any page script ran.
(function () {
  'use strict';
  const X = globalThis.__sx_ext;
  delete globalThis.__sx_ext;
  if (!X) return;
  const { H, report } = X;
  const EXT = 35;
  const key = H(EXT, 'key');
  if (!key) return;
  const stringify = JSON.stringify, parse = JSON.parse;
  // Message contents cross the browser as JSON text, so they arrive as they were sent
  // (the browser's own JSON would put an object's keys in order).
  const enc = v => (v === undefined ? undefined : stringify(v));
  const dec = t => (typeof t === 'string' ? parse(t) : undefined);
  const realms = new Map();
  const waiting = new Map();
  let seq = 1;
  let lastError;

  function post(o) { H(EXT, 'post', stringify(o)); }
  function event() {
    const ls = [];
    return {
      addListener(f) { if (typeof f === 'function' && !ls.includes(f)) ls.push(f); },
      removeListener(f) { const i = ls.indexOf(f); if (i >= 0) ls.splice(i, 1); },
      hasListener(f) { return ls.includes(f); },
      hasListeners() { return ls.length > 0; },
      _fire(...a) { const out = []; for (const f of ls.slice()) { try { out.push(f(...a)); } catch (e) { report(e); } } return out; },
    };
  }
  // A Chrome API call: a Promise, or the callback when the last argument is one.
  function finish(p, cb) {
    if (!cb) return p;
    p.then(v => { try { cb(v); } catch (e) { report(e); } },
      e => { lastError = { message: String(e && e.message || e) }; try { cb(); } catch (x) { report(x); } finally { lastError = undefined; } });
  }
  function api(fn) {
    return function (...a) {
      const cb = typeof a[a.length - 1] === 'function' ? a.pop() : null;
      let p;
      try { p = Promise.resolve(fn.apply(this, a)); } catch (e) { p = Promise.reject(e); }
      return finish(p, cb);
    };
  }
  // A request the browser answers later.
  function ask(o) { return new Promise((resolve, reject) => { const n = seq++; o.n = n; waiting.set(n, { resolve, reject }); post(o); }); }
  const done = () => undefined;

  function storageArea(r, name) {
    const S = (verb, k, v) => H(EXT, 'store', r.id, verb, name + ':' + (k == null ? '' : k), v == null ? '' : v);
    const names = () => { const t = S('keys'); return t ? t.split('\n') : []; };
    const read = k => { const v = S('get', k); if (v == null) return undefined; try { return parse(v); } catch (e) { return undefined; } };
    const area = {
      get: api(keys => {
        const out = {};
        if (keys == null) { for (const k of names()) { const v = read(k); if (v !== undefined) out[k] = v; } return out; }
        const want = typeof keys === 'string' ? { [keys]: undefined } : Array.isArray(keys) ? Object.fromEntries(keys.map(k => [k, undefined])) : keys;
        for (const k of Object.keys(want)) { const v = read(k); if (v !== undefined) out[k] = v; else if (want[k] !== undefined) out[k] = want[k]; }
        return out;
      }),
      set: api(items => {
        const changes = {};
        for (const k of Object.keys(items || {})) {
          const old = read(k), v = items[k];
          const text = stringify(v === undefined ? null : v);
          if (S('set', k, text) === '!') throw new Error('QUOTA_BYTES quota exceeded');
          changes[k] = { oldValue: old, newValue: parse(text) };
        }
        changed(r, name, changes, true);
      }),
      remove: api(keys => {
        const changes = {};
        for (const k of typeof keys === 'string' ? [keys] : keys || []) { const old = read(k); if (old !== undefined) { S('remove', k); changes[k] = { oldValue: old }; } }
        changed(r, name, changes, true);
      }),
      clear: api(() => {
        const changes = {};
        for (const k of names()) { changes[k] = { oldValue: read(k) }; S('remove', k); }
        changed(r, name, changes, true);
      }),
      getBytesInUse: api(() => names().reduce((n, k) => n + k.length + (S('get', k) || '').length, 0)),
      setAccessLevel: api(done),
      onChanged: event(),
      QUOTA_BYTES: 10485760,
    };
    return area;
  }
  function changed(r, area, changes, here) {
    if (!Object.keys(changes).length) return;
    r.storage.onChanged._fire(changes, area);
    if (r.storage[area]) r.storage[area].onChanged._fire(changes);
    if (here) post({ t: 'changed', id: r.id, area, changes: enc(changes) });
  }

  function make(id, manifest, messages, where) {
    const r = { id, where };
    const url = p => 'kaleidoscope-extension://' + id + '/' + String(p).replace(/^\//, '');
    const tab = () => { try { return parse(H(EXT, 'tab') || 'null'); } catch (e) { return null; } };
    const runtime = {
      id,
      getURL: url,
      getManifest: () => manifest,
      get lastError() { return lastError; },
      sendMessage: api((...a) => {
        if (a.length >= 2 && typeof a[0] === 'string' && (a.length > 2 || typeof a[1] !== 'object' || a[1] === null || a[0] === id)) a.shift();
        return ask({ t: 'send', id, m: enc(a[0]), from: where });
      }),
      onMessage: event(), onMessageExternal: event(), onInstalled: event(), onStartup: event(), onSuspend: event(),
      onConnect: event(), onUpdateAvailable: event(),
      connect: () => { const port = { name: '', onMessage: event(), onDisconnect: event(), postMessage() {}, disconnect() {} }; return port; },
      getPlatformInfo: api(() => ({ os: 'linux', arch: 'x86-64', nacl_arch: 'x86-64' })),
      openOptionsPage: api(() => { if (manifest.options_page || manifest.options_ui) post({ t: 'open', id, url: url(manifest.options_page || manifest.options_ui.page) }); }),
      setUninstallURL: api(done),
      reload: () => post({ t: 'reload', id }),
      getContexts: api(() => []),
    };
    const i18n = {
      getMessage(name, subs) {
        const m = Object.entries(messages).find(([k]) => k.toLowerCase() === String(name).toLowerCase());
        if (!m) return '';
        const list = subs == null ? [] : Array.isArray(subs) ? subs : [subs];
        let text = String(m[1].message || '');
        const ph = m[1].placeholders || {};
        text = text.replace(/\$([A-Za-z0-9_@]+)\$/g, (all, p) => { const e = Object.entries(ph).find(([k]) => k.toLowerCase() === p.toLowerCase()); return e ? String(e[1].content || '') : all; });
        return text.replace(/\$(\d)/g, (all, d) => list[+d - 1] == null ? '' : String(list[+d - 1]));
      },
      getUILanguage: () => 'en-US',
      getAcceptLanguages: api(() => ['en-US', 'en']),
      detectLanguage: api(() => ({ isReliable: false, languages: [] })),
    };
    const storage = { onChanged: event() };
    r.storage = storage;
    for (const name of ['local', 'sync', 'session', 'managed']) storage[name] = storageArea(r, name);
    const tabs = {
      TAB_ID_NONE: -1,
      query: api(() => { const t = tab(); return t ? [t] : []; }),
      get: api(() => tab()),
      getCurrent: api(() => (where === 'content' ? undefined : tab())),
      sendMessage: api((tabId, m) => ask({ t: 'tabsend', id, m: enc(m) })),
      create: api(o => { post({ t: 'open', id, url: o && o.url ? String(o.url) : '' }); return tab(); }),
      update: api((a, b) => { const o = typeof a === 'object' ? a : b; if (o && o.url) post({ t: 'open', id, url: String(o.url), here: true }); return tab(); }),
      reload: api(() => post({ t: 'reload-tab', id })),
      remove: api(done),
      onUpdated: event(), onActivated: event(), onRemoved: event(), onCreated: event(), onReplaced: event(),
    };
    const results = () => [{ frameId: 0, documentId: '0', result: null }];
    const scripting = {
      executeScript: api(o => ask({ t: 'exec', id, files: o.files || [], code: o.func ? '(' + String(o.func) + ')(...' + stringify(o.args || []) + ')' : '' }).then(results)),
      insertCSS: api(o => ask({ t: 'css', id, files: o.files || [], css: o.css || '' })),
      removeCSS: api(done),
      registerContentScripts: api(list => ask({ t: 'register', id, scripts: list || [] })),
      updateContentScripts: api(list => ask({ t: 'register', id, scripts: list || [] })),
      unregisterContentScripts: api(f => ask({ t: 'unregister', id, ids: f && f.ids || null })),
      getRegisteredContentScripts: api(f => ask({ t: 'registered', id, ids: f && f.ids || null })),
    };
    const dnr = {
      MAX_NUMBER_OF_ENABLED_STATIC_RULESETS: 50, MAX_NUMBER_OF_STATIC_RULESETS: 100, MAX_NUMBER_OF_DYNAMIC_RULES: 30000,
      MAX_NUMBER_OF_SESSION_RULES: 5000, GUARANTEED_MINIMUM_STATIC_RULES: 30000, MAX_NUMBER_OF_REGEX_RULES: 1000,
      getEnabledRulesets: api(() => ask({ t: 'rulesets', id })),
      updateEnabledRulesets: api(o => ask({ t: 'rulesets', id, enable: o.enableRulesetIds || [], disable: o.disableRulesetIds || [] }).then(done)),
      getDynamicRules: api(() => ask({ t: 'rules', id, which: 'dynamic' })),
      getSessionRules: api(() => ask({ t: 'rules', id, which: 'session' })),
      updateDynamicRules: api(o => ask({ t: 'rules', id, which: 'dynamic', add: o.addRules || [], remove: o.removeRuleIds || [] }).then(done)),
      updateSessionRules: api(o => ask({ t: 'rules', id, which: 'session', add: o.addRules || [], remove: o.removeRuleIds || [] }).then(done)),
      getAvailableStaticRuleCount: api(() => 30000),
      isRegexSupported: api(() => ({ isSupported: false, reason: 'syntaxError' })),
      setExtensionActionOptions: api(done),
      onRuleMatchedDebug: event(),
    };
    const action = {
      setBadgeText: api(done), getBadgeText: api(() => ''), setBadgeBackgroundColor: api(done), setBadgeTextColor: api(done),
      setIcon: api(done), setTitle: api(done), getTitle: api(() => manifest.name), setPopup: api(done), getPopup: api(() => ''),
      enable: api(done), disable: api(done), openPopup: api(done), getUserSettings: api(() => ({ isOnToolbar: true })),
      onClicked: event(),
    };
    const alarms = new Map();
    const chrome = {
      runtime, i18n, storage, tabs, scripting, action, declarativeNetRequest: dnr,
      browserAction: action, pageAction: action,
      extension: { getURL: url, inIncognitoContext: false, isAllowedIncognitoAccess: api(() => false), getBackgroundPage: () => (where === 'bg' ? globalThis : null), getViews: () => [] },
      permissions: {
        contains: api(() => true), request: api(() => true), remove: api(() => false),
        getAll: api(() => ({ permissions: manifest.permissions || [], origins: manifest.host_permissions || [] })),
        onAdded: event(), onRemoved: event(),
      },
      alarms: {
        create(name, info) {
          if (typeof name === 'object') { info = name; name = ''; }
          name = String(name || ''); info = info || {};
          chrome.alarms.clear(name);
          const due = info.when ? Math.max(0, info.when - Date.now()) : (info.delayInMinutes || info.periodInMinutes || 0) * 60000;
          const period = info.periodInMinutes ? info.periodInMinutes * 60000 : 0;
          const a = { name, scheduledTime: Date.now() + due, periodInMinutes: info.periodInMinutes };
          const fire = () => { if (alarms.get(name) !== a) return; a.scheduledTime = Date.now() + period; chrome.alarms.onAlarm._fire({ name, scheduledTime: a.scheduledTime }); if (period) a.timer = setTimeout(fire, period); else alarms.delete(name); };
          a.timer = setTimeout(fire, due);
          alarms.set(name, a);
          return Promise.resolve();
        },
        get: api(n => { const a = alarms.get(String(n || '')); return a && { name: a.name, scheduledTime: a.scheduledTime, periodInMinutes: a.periodInMinutes }; }),
        getAll: api(() => [...alarms.values()].map(a => ({ name: a.name, scheduledTime: a.scheduledTime, periodInMinutes: a.periodInMinutes }))),
        clear: api(n => { const a = alarms.get(String(n || '')); if (a) { clearTimeout(a.timer); alarms.delete(a.name); } return !!a; }),
        clearAll: api(() => { for (const a of alarms.values()) clearTimeout(a.timer); alarms.clear(); return true; }),
        onAlarm: event(),
      },
      contextMenus: { create: () => 0, update: api(done), remove: api(done), removeAll: api(done), onClicked: event() },
      commands: { getAll: api(() => []), onCommand: event() },
      webNavigation: { onBeforeNavigate: event(), onCommitted: event(), onDOMContentLoaded: event(), onCompleted: event(), onCreatedNavigationTarget: event(), onHistoryStateUpdated: event(), getFrame: api(() => null), getAllFrames: api(() => []) },
      management: { getSelf: api(() => ({ id, name: manifest.name, version: manifest.version, enabled: true, installType: 'normal', type: 'extension' })) },
      privacy: {},
      windows: { WINDOW_ID_CURRENT: -2, getCurrent: api(() => ({ id: 1, focused: true })), getAll: api(() => [{ id: 1, focused: true }]), onFocusChanged: event(), onRemoved: event(), onCreated: event() },
    };
    chrome.contextMenus.ContextType = {};
    chrome.menus = chrome.contextMenus;
    r.chrome = chrome;
    realms.set(id, r);
    return chrome;
  }

  // From the browser: an answer to `ask`, a message, or an event.
  X.onEvent(text => {
    let e;
    try { e = parse(text); } catch (x) { return 0; }
    if (e.k === 'reply') {
      const w = waiting.get(e.n);
      if (!w) return 0;
      waiting.delete(e.n);
      if (e.e) w.reject(new Error(e.e)); else w.resolve(dec(e.r));
      return 0;
    }
    const r = realms.get(e.id);
    if (!r) { if (e.k === 'msg') post({ t: 'reply', id: e.id, n: e.n, to: e.to, none: true }); return 0; }
    switch (e.k) {
      case 'msg': {
        let answered = false, pending = false;
        const respond = v => { if (answered) return; answered = true; post({ t: 'reply', id: e.id, n: e.n, to: e.to, r: enc(v) }); };
        for (const v of r.chrome.runtime.onMessage._fire(dec(e.m), e.sender || { id: e.id }, respond)) {
          if (v === true) pending = true;
          else if (v && typeof v.then === 'function') { pending = true; v.then(respond, x => { report(x); respond(undefined); }); }
        }
        if (!pending && !answered) post({ t: 'reply', id: e.id, n: e.n, to: e.to, none: true });
        return 0;
      }
      case 'changed': changed(r, e.area, dec(e.changes) || {}, false); return 0;
      case 'installed': r.chrome.runtime.onInstalled._fire({ reason: e.reason || 'install' }); return 0;
      case 'startup': r.chrome.runtime.onStartup._fire(); return 0;
      case 'eval': { try { (0, eval)(e.code); } catch (x) { report(x); } return 0; }
    }
    return 0;
  });

  Object.defineProperty(globalThis, '__kaleidoscopeExt', {
    value(id, k, manifest, messages, where) {
      if (k !== key || typeof id !== 'string') return undefined;
      return (realms.get(id) || { chrome: make(id, manifest || {}, messages || {}, where || 'content') }).chrome;
    },
  });
})();
