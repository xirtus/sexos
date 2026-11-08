// SPDX-License-Identifier: GPL-3.0-only
// The browser API for page scripts, built over one native call: __host(op, ...strings).
// Op numbers are crates/sex-js/src/lib.rs `op`. Nodes are numbers in the host's tree;
// each has at most one wrapper object here, so identity comparisons work.
(function () {
  'use strict';
  const H = globalThis.__host;
  delete globalThis.__host;
  const OP = { LOG: 1, BY_ID: 2, QUERY: 3, INFO: 4, GET_ATTR: 5, SET_ATTR: 6, REMOVE_ATTR: 7, GET_TEXT: 8, SET_TEXT: 9,
    GET_HTML: 10, SET_HTML: 11, CREATE_ELEMENT: 12, CREATE_TEXT: 13, INSERT: 14, DETACH: 15, PARENT: 16, CHILDREN: 17,
    ATTR_NAMES: 18, VIEWPORT: 19, NAVIGATE: 20, CLICKABLE: 21, RECT: 22, LOCATION: 23 };
  const nodes = new Map();

  function fmt(v) {
    if (typeof v === 'string') return v;
    if (v instanceof Error) return v.name + ': ' + v.message;
    if (v instanceof Node) return '<' + v.nodeName.toLowerCase() + '>';
    try { const s = JSON.stringify(v); return s === undefined ? String(v) : s; } catch (e) { return String(v); }
  }
  function log(level, args) { H(OP.LOG, level, Array.prototype.map.call(args, fmt).join(' ')); }
  function domError(msg) { const e = new Error(msg); e.name = 'DOMException'; return e; }
  function check(r) { if (r === '!') throw domError('The operation is not allowed here'); return r; }

  function wrap(id) {
    if (id === null || id === undefined || id === '') return null;
    id = +id;
    let n = nodes.get(id);
    if (n) return n;
    const info = H(OP.INFO, id);
    if (info === null) return null;
    const comma = info.indexOf(',');
    const type = +info.slice(0, comma), name = info.slice(comma + 1);
    if (type === 9) return document;
    n = type === 3 ? new Text(null, id) : new (ELEMENT_CLASSES[name] || HTMLElement)(id, name);
    nodes.set(id, n);
    return n;
  }
  function list(s) { return s ? s.split(',').map(wrap) : []; }
  function toNode(v) { return v instanceof Node ? v : document.createTextNode(String(v)); }

  // ---- Events ----
  class Event {
    constructor(type, init) {
      init = init || {};
      this.type = String(type);
      this.bubbles = !!init.bubbles;
      this.cancelable = !!init.cancelable;
      this.defaultPrevented = false;
      this.target = null;
      this.currentTarget = null;
      this.timeStamp = performance.now();
      this._stop = false;
      this._stopNow = false;
    }
    preventDefault() { if (this.cancelable) this.defaultPrevented = true; }
    stopPropagation() { this._stop = true; }
    stopImmediatePropagation() { this._stop = true; this._stopNow = true; }
  }
  class UIEvent extends Event {}
  class MouseEvent extends UIEvent {
    constructor(type, init) { super(type, init); init = init || {}; this.button = init.button || 0; this.clientX = init.clientX || 0; this.clientY = init.clientY || 0; }
  }
  class KeyboardEvent extends UIEvent {
    constructor(type, init) {
      super(type, init); init = init || {};
      this.key = init.key || ''; this.code = init.code || '';
      this.ctrlKey = !!init.ctrlKey; this.shiftKey = !!init.shiftKey; this.altKey = !!init.altKey; this.metaKey = !!init.metaKey;
      this.repeat = false;
    }
  }
  class CustomEvent extends Event { constructor(type, init) { super(type, init); this.detail = init && init.detail !== undefined ? init.detail : null; } }

  class EventTarget {
    addEventListener(type, fn, opts) {
      if (typeof fn !== 'function' && !(fn && typeof fn.handleEvent === 'function')) return;
      const l = this._listeners || (this._listeners = {});
      const arr = l[type] || (l[type] = []);
      const once = typeof opts === 'object' && opts !== null && !!opts.once;
      if (!arr.some(x => x.fn === fn)) arr.push({ fn, once });
      if (type === 'click' && this instanceof Element) H(OP.CLICKABLE, this._id);
    }
    removeEventListener(type, fn) {
      const arr = this._listeners && this._listeners[type];
      if (arr) { const i = arr.findIndex(x => x.fn === fn); if (i >= 0) arr.splice(i, 1); }
    }
    dispatchEvent(ev) {
      ev.target = this;
      const path = [];
      for (let n = this; n; n = n === document ? window : n === window ? null : n.parentNode) path.push(n);
      if (path[path.length - 1] !== window && this !== window) { /* detached: no window */ }
      for (let i = 0; i < path.length; i++) {
        const n = path[i];
        if (i > 0 && !ev.bubbles) break;
        ev.currentTarget = n;
        invoke(n, ev);
        if (ev._stop) break;
      }
      ev.currentTarget = null;
      return !ev.defaultPrevented;
    }
  }
  function invoke(n, ev) {
    const arr = n._listeners && n._listeners[ev.type];
    if (arr) {
      for (const x of arr.slice()) {
        if (x.once) n.removeEventListener(ev.type, x.fn);
        try { typeof x.fn === 'function' ? x.fn.call(n, ev) : x.fn.handleEvent(ev); } catch (e) { report(e); }
        if (ev._stopNow) return;
      }
    }
    // Handler properties (el.onclick = fn) and attributes (onclick="...").
    const prop = n['on' + ev.type];
    if (typeof prop === 'function') {
      try { if (prop.call(n, ev) === false) ev.preventDefault(); } catch (e) { report(e); }
    } else if (n instanceof Element) {
      const code = n.getAttribute('on' + ev.type);
      if (code) {
        try { if (new Function('event', code).call(n, ev) === false) ev.preventDefault(); } catch (e) { report(e); }
      }
    }
  }
  function report(e) { log(2, [e && e.stack ? e.name + ': ' + e.message + '\n' + e.stack : e]); }

  // ---- Nodes ----
  class Node extends EventTarget {
    constructor(id) { super(); this._id = id; }
    get nodeType() { return 1; }
    get ownerDocument() { return document; }
    get parentNode() { return wrap(H(OP.PARENT, this._id)); }
    get parentElement() { const p = this.parentNode; return p && p.nodeType === 1 ? p : null; }
    get childNodes() { return list(H(OP.CHILDREN, this._id)); }
    get firstChild() { const c = this.childNodes; return c[0] || null; }
    get lastChild() { const c = this.childNodes; return c[c.length - 1] || null; }
    hasChildNodes() { return this.childNodes.length > 0; }
    _sibling(d, elementsOnly) {
      const p = this.parentNode; if (!p) return null;
      const c = elementsOnly ? p.children : p.childNodes; const i = c.indexOf(this);
      return c[i + d] || null;
    }
    get nextSibling() { return this._sibling(1, false); }
    get previousSibling() { return this._sibling(-1, false); }
    get isConnected() { for (let n = this; n; n = n.parentNode) if (n === document) return true; return false; }
    get textContent() { return H(OP.GET_TEXT, this._id); }
    set textContent(v) { H(OP.SET_TEXT, this._id, v === null || v === undefined ? '' : String(v)); }
    get nodeValue() { return null; }
    appendChild(c) { return this.insertBefore(c, null); }
    insertBefore(c, ref) {
      if (c instanceof DocumentFragment) { for (const k of c.childNodes) this.insertBefore(k, ref); return c; }
      check(H(OP.INSERT, this._id, c._id, ref ? ref._id : ''));
      return c;
    }
    removeChild(c) { if (c.parentNode !== this) throw domError('Not a child of this node'); H(OP.DETACH, c._id); return c; }
    replaceChild(n, old) { this.insertBefore(n, old); return this.removeChild(old); }
    contains(o) { for (let n = o; n; n = n.parentNode) if (n === this) return true; return false; }
    cloneNode(deep) {
      if (this.nodeType === 3) return document.createTextNode(this.data);
      const holder = document.createElement('div');
      holder.innerHTML = this.outerHTML;
      const c = holder.firstChild;
      if (c) { H(OP.DETACH, c._id); if (!deep) c.textContent = ''; }
      return c;
    }
    getRootNode() { let n = this; while (n.parentNode) n = n.parentNode; return n; }
  }
  class CharacterData extends Node {
    get data() { return this.textContent; }
    set data(v) { this.textContent = v; }
    get nodeValue() { return this.data; }
    set nodeValue(v) { this.data = v; }
    get length() { return this.data.length; }
  }
  class Text extends CharacterData {
    constructor(text, id) {
      super(id === undefined ? +H(OP.CREATE_TEXT, text === undefined ? '' : String(text)) : id);
      if (id === undefined) nodes.set(this._id, this);
    }
    get nodeType() { return 3; }
    get nodeName() { return '#text'; }
    get wholeText() { return this.data; }
  }

  // Shared by Element, Document and DocumentFragment.
  const ParentMixin = {
    get children() { return this.childNodes.filter(n => n.nodeType === 1); },
    get childElementCount() { return this.children.length; },
    get firstElementChild() { return this.children[0] || null; },
    get lastElementChild() { const c = this.children; return c[c.length - 1] || null; },
    querySelector(s) { return wrap(check(H(OP.QUERY, this._id, String(s), '0'))); },
    querySelectorAll(s) { return list(check(H(OP.QUERY, this._id, String(s), '1'))); },
    getElementsByTagName(t) { return this.querySelectorAll(t === '*' ? '*' : String(t).toLowerCase()); },
    getElementsByClassName(c) { return this.querySelectorAll(String(c).trim().split(/\s+/).map(x => '.' + x).join('')); },
    append(...xs) { for (const x of xs) this.appendChild(toNode(x)); },
    prepend(...xs) { const f = this.firstChild; for (const x of xs) this.insertBefore(toNode(x), f); },
    replaceChildren(...xs) { this.textContent = ''; this.append(...xs); },
  };
  function mix(cls, obj) { Object.defineProperties(cls.prototype, Object.getOwnPropertyDescriptors(obj)); }

  class DOMTokenList {
    constructor(el, attr) { this._el = el; this._attr = attr; }
    _get() { return (this._el.getAttribute(this._attr) || '').split(/\s+/).filter(Boolean); }
    _set(a) { this._el.setAttribute(this._attr, a.join(' ')); }
    get length() { return this._get().length; }
    get value() { return this._el.getAttribute(this._attr) || ''; }
    item(i) { return this._get()[i] || null; }
    contains(t) { return this._get().includes(t); }
    add(...ts) { const a = this._get(); for (const t of ts) if (!a.includes(t)) a.push(t); this._set(a); }
    remove(...ts) { this._set(this._get().filter(x => !ts.includes(x))); }
    toggle(t, force) {
      const has = this.contains(t);
      const want = force === undefined ? !has : !!force;
      if (want && !has) this.add(t); else if (!want && has) this.remove(t);
      return want;
    }
    replace(a, b) { const l = this._get(); const i = l.indexOf(a); if (i < 0) return false; l[i] = b; this._set(l); return true; }
    forEach(f, t) { this._get().forEach(f, t); }
    [Symbol.iterator]() { return this._get()[Symbol.iterator](); }
    toString() { return this.value; }
  }

  function camelToKebab(p) { return p.startsWith('--') ? p : p.replace(/[A-Z]/g, m => '-' + m.toLowerCase()).replace(/^webkit-/, '-webkit-'); }
  function parseStyle(s) {
    const out = new Map();
    for (const part of (s || '').split(';')) {
      const i = part.indexOf(':');
      if (i > 0) out.set(part.slice(0, i).trim().toLowerCase(), part.slice(i + 1).trim());
    }
    return out;
  }
  function styleFor(el) {
    const read = () => parseStyle(el.getAttribute('style'));
    const write = m => { const s = [...m].map(([k, v]) => k + ': ' + v).join('; '); if (s) el.setAttribute('style', s); else el.removeAttribute('style'); };
    const api = {
      getPropertyValue: p => read().get(String(p).toLowerCase()) || '',
      setProperty: (p, v) => { const m = read(); p = String(p).toLowerCase(); if (v === null || v === '') m.delete(p); else m.set(p, String(v)); write(m); },
      removeProperty: p => { const m = read(); p = String(p).toLowerCase(); const old = m.get(p) || ''; m.delete(p); write(m); return old; },
    };
    return new Proxy(api, {
      get(t, p) {
        if (typeof p !== 'string') return undefined;
        if (p in t) return t[p];
        if (p === 'cssText') return el.getAttribute('style') || '';
        if (p === 'length') return read().size;
        return read().get(camelToKebab(p)) || '';
      },
      set(t, p, v) {
        if (typeof p !== 'string') return true;
        if (p === 'cssText') { el.setAttribute('style', String(v)); return true; }
        api.setProperty(camelToKebab(p), v === null || v === undefined ? '' : typeof v === 'number' ? String(v) : String(v));
        return true;
      },
    });
  }
  function datasetFor(el) {
    const key = p => 'data-' + camelToKebab(p);
    return new Proxy({}, {
      get(t, p) { if (typeof p !== 'string') return undefined; const v = el.getAttribute(key(p)); return v === null ? undefined : v; },
      set(t, p, v) { if (typeof p === 'string') el.setAttribute(key(p), String(v)); return true; },
      deleteProperty(t, p) { if (typeof p === 'string') el.removeAttribute(key(p)); return true; },
      has(t, p) { return typeof p === 'string' && el.hasAttribute(key(p)); },
    });
  }

  class Element extends Node {
    constructor(id, tag) { super(id); this._tag = tag; }
    get nodeName() { return this._tag; }
    get tagName() { return this._tag; }
    get localName() { return this._tag.toLowerCase(); }
    getAttribute(n) { return H(OP.GET_ATTR, this._id, String(n)); }
    setAttribute(n, v) { check(H(OP.SET_ATTR, this._id, String(n), String(v))); }
    removeAttribute(n) { H(OP.REMOVE_ATTR, this._id, String(n)); }
    hasAttribute(n) { return this.getAttribute(n) !== null; }
    toggleAttribute(n, force) {
      const has = this.hasAttribute(n); const want = force === undefined ? !has : !!force;
      if (want && !has) this.setAttribute(n, ''); else if (!want && has) this.removeAttribute(n);
      return want;
    }
    getAttributeNames() { const s = H(OP.ATTR_NAMES, this._id); return s ? s.split('\n') : []; }
    get attributes() { return this.getAttributeNames().map(name => ({ name, value: this.getAttribute(name) })); }
    get id() { return this.getAttribute('id') || ''; }
    set id(v) { this.setAttribute('id', v); }
    get className() { return this.getAttribute('class') || ''; }
    set className(v) { this.setAttribute('class', v); }
    get classList() { return this._classList || (this._classList = new DOMTokenList(this, 'class')); }
    get style() { return this._style || (this._style = styleFor(this)); }
    set style(v) { this.setAttribute('style', v); }
    get dataset() { return this._dataset || (this._dataset = datasetFor(this)); }
    get innerHTML() { return H(OP.GET_HTML, this._id, '0'); }
    set innerHTML(v) { H(OP.SET_HTML, this._id, String(v)); }
    get outerHTML() { return H(OP.GET_HTML, this._id, '1'); }
    get innerText() { return this.textContent; }
    set innerText(v) { this.textContent = v; }
    get nextElementSibling() { return this._sibling(1, true); }
    get previousElementSibling() { return this._sibling(-1, true); }
    insertAdjacentHTML(where, html) {
      const holder = document.createElement('div');
      holder.innerHTML = html;
      const kids = holder.childNodes;
      const w = String(where).toLowerCase();
      for (const k of kids) {
        if (w === 'beforebegin') this.parentNode && this.parentNode.insertBefore(k, this);
        else if (w === 'afterbegin') this.insertBefore(k, this.firstChild);
        else if (w === 'beforeend') this.appendChild(k);
        else if (w === 'afterend') this.parentNode && this.parentNode.insertBefore(k, this.nextSibling);
      }
    }
    insertAdjacentElement(where, el) { this.insertAdjacentHTML(where, ''); return el; }
    remove() { H(OP.DETACH, this._id); }
    before(...xs) { const p = this.parentNode; if (p) for (const x of xs) p.insertBefore(toNode(x), this); }
    after(...xs) { const p = this.parentNode; if (!p) return; const next = this.nextSibling; for (const x of xs) p.insertBefore(toNode(x), next); }
    replaceWith(...xs) { this.before(...xs); this.remove(); }
    matches(sel) { const p = this.parentNode || document; return p.querySelectorAll(sel).includes(this) || document.querySelectorAll(sel).includes(this); }
    closest(sel) { for (let n = this; n && n.nodeType === 1; n = n.parentNode) if (n.matches(sel)) return n; return null; }
    getBoundingClientRect() {
      const [x, y, w, h] = H(OP.RECT, this._id).split(',').map(Number);
      const top = y - window.scrollY;
      return { x, y: top, left: x, top, width: w, height: h, right: x + w, bottom: top + h };
    }
    getClientRects() { return [this.getBoundingClientRect()]; }
    get offsetWidth() { return this.getBoundingClientRect().width; }
    get offsetHeight() { return this.getBoundingClientRect().height; }
    get offsetLeft() { return this.getBoundingClientRect().left; }
    get offsetTop() { return this.getBoundingClientRect().top + window.scrollY; }
    get clientWidth() { return this.offsetWidth; }
    get clientHeight() { return this.offsetHeight; }
    get scrollTop() { return 0; }
    set scrollTop(v) {}
    scrollIntoView() {}
    focus() { document._active = this; }
    blur() { if (document._active === this) document._active = null; }
    click() { this.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true })); }
  }
  mix(Element, ParentMixin);
  class HTMLElement extends Element {
    get hidden() { return this.hasAttribute('hidden'); }
    set hidden(v) { this.toggleAttribute('hidden', !!v); }
    get title() { return this.getAttribute('title') || ''; }
    set title(v) { this.setAttribute('title', v); }
    get tabIndex() { return +(this.getAttribute('tabindex') || 0); }
  }
  function reflect(cls, props) {
    for (const p of props) Object.defineProperty(cls.prototype, p, {
      get() { return this.getAttribute(p.toLowerCase()) || ''; },
      set(v) { this.setAttribute(p.toLowerCase(), v); },
      configurable: true,
    });
  }
  class HTMLAnchorElement extends HTMLElement {
    get href() { const h = this.getAttribute('href'); return h === null ? '' : resolveUrl(h); }
    set href(v) { this.setAttribute('href', v); }
  }
  reflect(HTMLAnchorElement, ['target', 'rel', 'download']);
  class HTMLImageElement extends HTMLElement {
    get src() { const h = this.getAttribute('src'); return h === null ? '' : resolveUrl(h); }
    set src(v) { this.setAttribute('src', v); }
    get complete() { return true; }
    get naturalWidth() { return this.offsetWidth; }
    get naturalHeight() { return this.offsetHeight; }
  }
  reflect(HTMLImageElement, ['alt', 'width', 'height']);
  class HTMLInputElement extends HTMLElement {
    get value() { return this._value !== undefined ? this._value : this.getAttribute('value') || ''; }
    set value(v) { this._value = String(v); this.setAttribute('value', v); }
    get checked() { return this.hasAttribute('checked'); }
    set checked(v) { this.toggleAttribute('checked', !!v); }
    get disabled() { return this.hasAttribute('disabled'); }
    set disabled(v) { this.toggleAttribute('disabled', !!v); }
    get type() { return (this.getAttribute('type') || 'text').toLowerCase(); }
  }
  reflect(HTMLInputElement, ['name', 'placeholder']);
  class HTMLButtonElement extends HTMLElement {
    get disabled() { return this.hasAttribute('disabled'); }
    set disabled(v) { this.toggleAttribute('disabled', !!v); }
    get type() { return (this.getAttribute('type') || 'submit').toLowerCase(); }
  }
  reflect(HTMLButtonElement, ['name', 'value']);
  class HTMLTemplateElement extends HTMLElement {
    get content() { const f = document.createDocumentFragment(); f.innerHTML = this.innerHTML; return f; }
  }
  class HTMLCanvasElement extends HTMLElement {
    get width() { return +(this.getAttribute('width') || 300); }
    set width(v) { this.setAttribute('width', v); }
    get height() { return +(this.getAttribute('height') || 150); }
    set height(v) { this.setAttribute('height', v); }
    getContext() { return null; }
  }
  const ELEMENT_CLASSES = { A: HTMLAnchorElement, IMG: HTMLImageElement, INPUT: HTMLInputElement, BUTTON: HTMLButtonElement,
    TEMPLATE: HTMLTemplateElement, CANVAS: HTMLCanvasElement, TEXTAREA: HTMLInputElement, SELECT: HTMLInputElement };

  // A fragment is a detached holder element; inserting it moves its children.
  class DocumentFragment extends Node {
    constructor() { super(+check(H(OP.CREATE_ELEMENT, 'sx-fragment'))); }
    get nodeType() { return 11; }
    get nodeName() { return '#document-fragment'; }
    get innerHTML() { return H(OP.GET_HTML, this._id, '0'); }
    set innerHTML(v) { H(OP.SET_HTML, this._id, String(v)); }
    getElementById(id) { return this.querySelector('#' + id); }
  }
  mix(DocumentFragment, ParentMixin);

  class Document extends Node {
    constructor() { super(0); this._active = null; this.readyState = 'loading'; }
    get nodeType() { return 9; }
    get nodeName() { return '#document'; }
    get documentElement() { return this.querySelector('html'); }
    get head() { return this.querySelector('head'); }
    get body() { return this.querySelector('body'); }
    get activeElement() { return this._active || this.body; }
    get title() { const t = this.querySelector('title'); return t ? t.textContent.trim() : ''; }
    set title(v) {
      let t = this.querySelector('title');
      if (!t) { t = this.createElement('title'); const h = this.head; if (h) h.appendChild(t); }
      t.textContent = v;
    }
    get URL() { return location.href; }
    get location() { return location; }
    get defaultView() { return window; }
    get cookie() { return ''; }
    set cookie(v) {}
    getElementById(id) { return wrap(H(OP.BY_ID, String(id))); }
    createElement(tag) {
      const id = +check(H(OP.CREATE_ELEMENT, String(tag)));
      return wrap(id);
    }
    createElementNS(ns, tag) { return this.createElement(tag); }
    createTextNode(t) { return new Text(t); }
    createComment() { return new Text(''); }
    createDocumentFragment() { return new DocumentFragment(); }
    createEvent() { return new Event(''); }
    hasFocus() { return true; }
  }
  mix(Document, ParentMixin);
  const document = new Document();
  nodes.set(0, document);

  // ---- URLs and location ----
  function resolveUrl(h) {
    h = String(h);
    if (/^[a-z][a-z0-9+.-]*:/i.test(h)) return h;
    const base = H(OP.LOCATION) || '';
    const m = /^([a-z]+:\/\/[^/]*)([^?#]*)/i.exec(base);
    if (!m) return h;
    if (h.startsWith('//')) return base.split(':')[0] + ':' + h;
    if (h.startsWith('/')) return m[1] + h;
    if (h.startsWith('#') || h.startsWith('?')) return base.replace(h.startsWith('#') ? /#.*$/ : /[?#].*$/, '') + h;
    return m[1] + m[2].replace(/[^/]*$/, '') + h;
  }
  const location = {
    get href() { return H(OP.LOCATION) || ''; },
    set href(v) { H(OP.NAVIGATE, resolveUrl(v)); },
    assign(v) { H(OP.NAVIGATE, resolveUrl(v)); },
    replace(v) { H(OP.NAVIGATE, resolveUrl(v)); },
    reload() { H(OP.NAVIGATE, this.href); },
    get protocol() { const m = /^([a-z]+:)/i.exec(this.href); return m ? m[1] : ''; },
    get host() { const m = /^[a-z]+:\/\/([^/?#]*)/i.exec(this.href); return m ? m[1] : ''; },
    get hostname() { return this.host.replace(/:\d+$/, ''); },
    get pathname() { const m = /^[a-z]+:\/\/[^/]*([^?#]*)/i.exec(this.href); return (m && m[1]) || '/'; },
    get search() { const m = /\?[^#]*/.exec(this.href); return m ? m[0] : ''; },
    get hash() { const m = /#.*/.exec(this.href); return m ? m[0] : ''; },
    get origin() { const m = /^[a-z]+:\/\/[^/?#]*/i.exec(this.href); return m ? m[0] : 'null'; },
    toString() { return this.href; },
  };

  // ---- Timers ----
  let timerSeq = 0;
  const timers = new Map();
  let frames = [];
  function addTimer(fn, ms, args, repeat) {
    const id = ++timerSeq;
    ms = Math.max(0, +ms || 0);
    timers.set(id, { fn, args, due: performance.now() + ms, every: repeat ? Math.max(ms, 4) : 0 });
    return id;
  }
  function runTimers(now) {
    // Each timer due now runs once per tick, in due order.
    const due = [...timers.entries()].filter(([, t]) => t.due <= now).sort((a, b) => a[1].due - b[1].due);
    for (const [id, t] of due) {
      if (!timers.has(id)) continue;
      if (t.every) t.due = Math.max(t.due + t.every, now); else timers.delete(id);
      try { typeof t.fn === 'function' ? t.fn(...t.args) : (0, eval)(String(t.fn)); } catch (e) { report(e); }
    }
    if (frames.length) {
      const run = frames; frames = [];
      for (const f of run) if (f.fn) { try { f.fn(now); } catch (e) { report(e); } }
    }
    let next = -1;
    for (const t of timers.values()) { const w = Math.max(0, Math.ceil(t.due - performance.now())); if (next < 0 || w < next) next = w; }
    if (frames.length) next = next < 0 ? 16 : Math.min(next, 16);
    return next;
  }

  const storage = () => {
    const m = new Map();
    return {
      getItem: k => (m.has(String(k)) ? m.get(String(k)) : null),
      setItem: (k, v) => { m.set(String(k), String(v)); },
      removeItem: k => { m.delete(String(k)); },
      clear: () => m.clear(),
      key: i => [...m.keys()][i] || null,
      get length() { return m.size; },
    };
  };

  const startNs = Date.now();
  const performance = { now: () => Date.now() - startNs, timeOrigin: startNs, mark() {}, measure() {} };
  const console = {
    log() { log(0, arguments); }, info() { log(0, arguments); }, debug() { log(0, arguments); },
    warn() { log(1, arguments); }, error() { log(2, arguments); },
    assert(c, ...rest) { if (!c) log(2, ['Assertion failed:', ...rest]); },
    table(x) { log(0, [x]); }, group() {}, groupEnd() {}, time() {}, timeEnd() {}, trace() { log(0, arguments); },
  };

  const window = globalThis;
  const view = () => H(OP.VIEWPORT).split(',').map(Number);
  Object.setPrototypeOf(window, EventTarget.prototype);
  const globals = {
    window, self: window, globalThis: window, document, location, console, performance,
    navigator: { userAgent: 'Mozilla/5.0 (SexOS) Web/0.2', language: 'en', languages: ['en'], platform: 'SexOS', onLine: true },
    screen: { get width() { return view()[0]; }, get height() { return view()[1]; } },
    history: { length: 1, state: null, pushState() {}, replaceState() {}, back() {}, forward() {}, go() {} },
    localStorage: storage(), sessionStorage: storage(),
    setTimeout: (fn, ms, ...args) => addTimer(fn, ms, args, false),
    setInterval: (fn, ms, ...args) => addTimer(fn, ms, args, true),
    clearTimeout: id => { timers.delete(id); },
    clearInterval: id => { timers.delete(id); },
    requestAnimationFrame: fn => { const f = { id: ++timerSeq, fn }; frames.push(f); return f.id; },
    cancelAnimationFrame: id => { for (const f of frames) if (f.id === id) f.fn = null; },
    queueMicrotask: fn => { Promise.resolve().then(fn); },
    alert: m => log(1, ['alert:', m === undefined ? '' : m]),
    confirm: () => false, prompt: () => null,
    getComputedStyle: el => el.style,
    matchMedia: q => ({ matches: false, media: q, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {} }),
    scrollTo() {}, scrollBy() {}, scroll() {}, focus() {}, blur() {},
    Event, UIEvent, MouseEvent, KeyboardEvent, CustomEvent, EventTarget, Node, Text, CharacterData, Element, HTMLElement,
    HTMLAnchorElement, HTMLImageElement, HTMLInputElement, HTMLButtonElement, HTMLCanvasElement, HTMLTemplateElement,
    Document, DocumentFragment, DOMTokenList,
    devicePixelRatio: 1, scrollX: 0, scrollY: 0, pageXOffset: 0, pageYOffset: 0, name: '',
  };
  for (const k of Object.keys(globals)) {
    Object.defineProperty(window, k, { value: globals[k], writable: true, configurable: true, enumerable: false });
  }
  Object.defineProperty(window, 'innerWidth', { get: () => view()[0], configurable: true });
  Object.defineProperty(window, 'innerHeight', { get: () => view()[1], configurable: true });
  Object.defineProperty(window, 'top', { get: () => window, configurable: true });
  Object.defineProperty(window, 'parent', { get: () => window, configurable: true });

  const KEY_TYPES = new Set(['keydown', 'keyup', 'keypress']);
  // The browser's entry point: one string, fields split by tabs.
  Object.defineProperty(window, '__sx_dispatch', { value: function (msg) {
    const f = String(msg).split('\t');
    switch (f[0]) {
      case 'tick': return runTimers(performance.now());
      case 'scroll': window.scrollY = window.pageYOffset = +f[1] || 0; return 0;
      case 'click': {
        const el = wrap(f[1]);
        if (!el) return 0;
        const ev = new MouseEvent('click', { bubbles: true, cancelable: true });
        el.dispatchEvent(ev);
        return ev.defaultPrevented ? 1 : 0;
      }
      case 'key': {
        if (!KEY_TYPES.has(f[1])) return 0;
        const ev = new KeyboardEvent(f[1], { key: f[2] || '', code: f[3] || '', bubbles: true, cancelable: true, ctrlKey: f[4] === '1' });
        (document._active || document.body || document).dispatchEvent(ev);
        return ev.defaultPrevented ? 1 : 0;
      }
      case 'ready': {
        document.readyState = 'interactive';
        document.dispatchEvent(new Event('DOMContentLoaded', { bubbles: true }));
        document.readyState = 'complete';
        window.dispatchEvent(new Event('load'));
        return 0;
      }
    }
    return 0;
  }, configurable: false, writable: false, enumerable: false });
})();
