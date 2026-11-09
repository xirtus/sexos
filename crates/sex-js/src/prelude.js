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
    ATTR_NAMES: 18, VIEWPORT: 19, NAVIGATE: 20, CLICKABLE: 21, RECT: 22, LOCATION: 23, CANVAS: 24, MEASURE: 25, HIT: 26, VALUE: 27, CARET: 28, GL: 64, GL_QUERY: 65, FETCH: 66, FETCH_BODY: 67, FETCH_HEAD: 68, COOKIE: 69 };
  const nodes = new Map();
  const defaultChecked = new WeakMap();

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
    constructor(type, init) {
      super(type, init); init = init || {};
      this.button = init.button || 0; this.buttons = init.buttons || 0;
      this.clientX = init.clientX || 0; this.clientY = init.clientY || 0;
      this.screenX = this.clientX; this.screenY = this.clientY;
      this.movementX = init.movementX || 0; this.movementY = init.movementY || 0;
      this.ctrlKey = !!init.ctrlKey; this.shiftKey = !!init.shiftKey; this.altKey = !!init.altKey; this.metaKey = !!init.metaKey;
      this.relatedTarget = init.relatedTarget || null;
      this.detail = init.detail || 0;
      this.view = window;
    }
    get x() { return this.clientX; }
    get y() { return this.clientY; }
    get pageX() { return this.clientX + window.scrollX; }
    get pageY() { return this.clientY + window.scrollY; }
    // Relative to the target's box, as games with a canvas use them.
    get offsetX() { const t = this.target; return t && t.getBoundingClientRect ? this.clientX - t.getBoundingClientRect().left : this.clientX; }
    get offsetY() { const t = this.target; return t && t.getBoundingClientRect ? this.clientY - t.getBoundingClientRect().top : this.clientY; }
    getModifierState() { return false; }
  }
  class PointerEvent extends MouseEvent {
    constructor(type, init) {
      super(type, init); init = init || {};
      this.pointerId = init.pointerId || 1; this.pointerType = init.pointerType || 'mouse'; this.isPrimary = init.isPrimary !== false;
      this.width = 1; this.height = 1; this.pressure = this.buttons ? 0.5 : 0; this.tiltX = 0; this.tiltY = 0;
    }
    getCoalescedEvents() { return [this]; }
    getPredictedEvents() { return []; }
  }
  class WheelEvent extends MouseEvent {
    constructor(type, init) { super(type, init); init = init || {}; this.deltaX = init.deltaX || 0; this.deltaY = init.deltaY || 0; this.deltaZ = 0; this.deltaMode = init.deltaMode || 0; }
  }
  WheelEvent.DOM_DELTA_PIXEL = 0; WheelEvent.DOM_DELTA_LINE = 1; WheelEvent.DOM_DELTA_PAGE = 2;
  class InputEvent extends UIEvent { constructor(type, init) { super(type, init); init = init || {}; this.data = init.data === undefined ? null : init.data; this.inputType = init.inputType || ''; this.isComposing = false; } }
  class SubmitEvent extends Event { constructor(type, init) { super(type, init); this.submitter = (init && init.submitter) || null; } }
  class FocusEvent extends UIEvent { constructor(type, init) { super(type, init); this.relatedTarget = (init && init.relatedTarget) || null; } }
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
      // A click runs its target's activation behaviour (checkboxes, submit buttons, labels).
      if (ev.type === 'click' && this.nodeType === 1 && !ev._activating) { ev._activating = true; return activate(this, ev); }
      return this._dispatch(ev);
    }
    _dispatch(ev) {
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
    focus() { setFocus(this); }
    blur() { if (document._active === this) setFocus(null); }
    click() { this.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, detail: 1 })); }
    setPointerCapture() { mouse.capture = this; }
    releasePointerCapture() { if (mouse.capture === this) mouse.capture = null; }
    hasPointerCapture() { return mouse.capture === this; }
    requestPointerLock() { mouse.lock = this; document.pointerLockElement = this; document.dispatchEvent(new Event('pointerlockchange')); }
  }
  mix(Element, ParentMixin);
  // el.onclick = fn makes the element clickable, as addEventListener does.
  Object.defineProperty(Element.prototype, 'onclick', {
    get() { return this._onclick || null; },
    set(fn) { this._onclick = typeof fn === 'function' ? fn : null; if (this._onclick) H(OP.CLICKABLE, this._id); },
    configurable: true,
  });
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
  // ---- Forms ----
  const TEXT_TYPES = new Set(['text', 'search', 'email', 'url', 'tel', 'password', 'number']);
  function isText(el) {
    if (!el || el.nodeType !== 1) return false;
    if (el.localName === 'textarea') return true;
    return el.localName === 'input' && TEXT_TYPES.has((el.getAttribute('type') || 'text').toLowerCase());
  }
  function focusable(el) {
    for (let n = el; n && n.nodeType === 1; n = n.parentNode) {
      const t = n.localName;
      if (t === 'input' || t === 'textarea' || t === 'select' || t === 'button' || (t === 'a' && n.hasAttribute('href')) || n.hasAttribute('tabindex'))
        return n.hasAttribute('disabled') ? null : n;
    }
    return null;
  }
  // The text control being edited: its caret (in code points) and its value at focus.
  const edit = { el: null, caret: 0, start: '' };
  function showCaret() { if (edit.el && document._active === edit.el) H(OP.CARET, edit.el._id, String(edit.caret)); else H(OP.CARET); }
  function setFocus(el) {
    const prev = document._active;
    if (prev === el) return;
    if (prev && edit.el === prev && prev.value !== edit.start) prev.dispatchEvent(new Event('change', { bubbles: true }));
    document._active = el;
    edit.el = null;
    if (prev && prev.dispatchEvent) { prev.dispatchEvent(new FocusEvent('blur', { relatedTarget: el })); prev.dispatchEvent(new FocusEvent('focusout', { bubbles: true, relatedTarget: el })); }
    if (el && document._active === el) {
      el.dispatchEvent(new FocusEvent('focus', { relatedTarget: prev })); el.dispatchEvent(new FocusEvent('focusin', { bubbles: true, relatedTarget: prev }));
    }
    if (isText(document._active)) { edit.el = document._active; edit.start = edit.el.value; edit.caret = Array.from(edit.start).length; }
    showCaret();
  }
  function fire(el, type, Kind, init) { const ev = new (Kind || Event)(type, Object.assign({ bubbles: true }, init)); el.dispatchEvent(ev); return ev; }
  function formOf(el) {
    const id = el.getAttribute('form');
    if (id) { const f = document.getElementById(id); if (f && f.localName === 'form') return f; }
    return el.closest('form');
  }
  // Typing into the focused text control. Answers true when the key was used.
  function editKey(el, key, ctrl) {
    const chars = Array.from(el.value);
    let c = Math.min(edit.caret, chars.length), insert = null, removed = null, type = '';
    const multi = el.localName === 'textarea';
    const lineStart = i => { while (i > 0 && chars[i - 1] !== '\n') i--; return i; };
    const lineEnd = i => { while (i < chars.length && chars[i] !== '\n') i++; return i; };
    switch (key) {
      case 'ArrowLeft': c = Math.max(0, c - 1); break;
      case 'ArrowRight': c = Math.min(chars.length, c + 1); break;
      case 'Home': c = multi ? lineStart(c) : 0; break;
      case 'End': c = multi ? lineEnd(c) : chars.length; break;
      case 'ArrowUp': case 'ArrowDown': {
        if (!multi) { c = key === 'ArrowUp' ? 0 : chars.length; break; }
        const s0 = lineStart(c), col = c - s0;
        if (key === 'ArrowUp') { if (s0 === 0) { c = 0; break; } const p0 = lineStart(s0 - 1); c = Math.min(p0 + col, s0 - 1); }
        else { const e0 = lineEnd(c); if (e0 >= chars.length) { c = chars.length; break; } const n0 = e0 + 1; c = Math.min(n0 + col, lineEnd(n0)); }
        break;
      }
      case 'Backspace': if (c > 0) { removed = [c - 1, 1]; type = 'deleteContentBackward'; } break;
      case 'Delete': if (c < chars.length) { removed = [c, 1]; type = 'deleteContentForward'; } break;
      case 'Enter':
        if (multi) { insert = '\n'; type = 'insertLineBreak'; break; }
        // Enter commits the value (change), then submits the form.
        if (el.value !== edit.start) { edit.start = el.value; fire(el, 'change'); }
        { const f = formOf(el); if (f) submitForm(f, null, true); }
        return true;
      default:
        if (ctrl || Array.from(key).length !== 1) return false;
        insert = key; type = 'insertText';
    }
    if (insert !== null || removed) {
      const max = el.hasAttribute('maxlength') ? +el.getAttribute('maxlength') : -1;
      if (insert !== null && max >= 0 && chars.length >= max) { edit.caret = c; showCaret(); return true; }
      if (el.hasAttribute('readonly')) return true;
      if (!fire(el, 'beforeinput', InputEvent, { cancelable: true, data: insert, inputType: type }).defaultPrevented) {
        if (removed) { chars.splice(removed[0], removed[1]); c = removed[0]; }
        if (insert !== null) { chars.splice(c, 0, insert); c += 1; }
        H(OP.VALUE, el._id, chars.join(''));
        edit.caret = c;
        fire(el, 'input', InputEvent, { data: insert, inputType: type });
      }
    }
    edit.caret = c;
    showCaret();
    return true;
  }
  // What a successful submission of `form` sends: [name, value] pairs.
  function formEntries(form, submitter) {
    const out = [];
    for (const el of form.elements) {
      const name = el.getAttribute('name');
      if (!name || disabledControl(el)) continue;
      const t = el.localName === 'input' ? el.type : el.localName;
      if ((t === 'checkbox' || t === 'radio') && !el.checked) continue;
      if (t === 'submit' || t === 'image' || t === 'reset' || t === 'button' || (el.localName === 'button' && el !== submitter)) { if (el !== submitter) continue; }
      if (t === 'file') continue;
      if ((t === 'checkbox' || t === 'radio') && !el.hasAttribute('value')) { out.push([name, 'on']); continue; }
      out.push([name, el.value]);
    }
    return out;
  }
  function submitForm(form, submitter, events) {
    if (events) {
      if (!form.checkValidity()) return;
      const ev = new SubmitEvent('submit', { bubbles: true, cancelable: true, submitter });
      form.dispatchEvent(ev);
      if (ev.defaultPrevented) return;
    }
    const attr = (n, f) => (submitter && submitter.getAttribute(f)) || form.getAttribute(n);
    const method = (attr('method', 'formmethod') || 'get').toLowerCase();
    if (method === 'dialog') return;
    const action = resolveUrl(attr('action', 'formaction') || H(OP.LOCATION) || '');
    const enc = v => encodeURIComponent(String(v).replace(/\r?\n|\r/g, '\r\n')).replace(/[!'()~]/g, c => '%' + c.charCodeAt(0).toString(16).toUpperCase()).replace(/%20/g, '+');
    const query = formEntries(form, submitter).map(([k, v]) => enc(k) + '=' + enc(v)).join('&');
    if (method === 'post') { H(OP.NAVIGATE, action, 'POST', query); return; }
    H(OP.NAVIGATE, action.replace(/[?#].*$/, '') + '?' + query);
  }
  // A click on `el`: checkboxes and radios change before the click and change back if
  // it is cancelled; submit and reset buttons, selects and labels act after it.
  function activate(target, ev) {
    // A click inside a button (on its text, say) activates the button; the event still
    // goes to what was clicked.
    const inner = target.localName;
    const el = (inner !== 'input' && inner !== 'button' && inner !== 'select' && inner !== 'textarea' && target.closest('button')) || target;
    const t = el.localName, type = t === 'input' ? el.type : '';
    if (el.hasAttribute('disabled') && (t === 'input' || t === 'button' || t === 'select' || t === 'textarea')) return true;
    let undo = null, changed = true;
    if (type === 'checkbox') { const was = el.checked; el.checked = !was; undo = () => { el.checked = was; }; }
    else if (type === 'radio' && !el.checked) {
      const group = el.name ? radios(el) : [];
      const was = group.find(r => r.checked);
      for (const r of group) r.checked = false;
      el.checked = true;
      undo = () => { el.checked = false; if (was) was.checked = true; };
    } else if (type === 'radio') { undo = () => {}; changed = false; }
    const ok = target._dispatch(ev);
    if (undo) {
      if (!ok) { undo(); return ok; }
      if (changed) { fire(el, 'input', InputEvent); fire(el, 'change'); }
      return ok;
    }
    if (!ok) return ok;
    const button = t === 'button' ? (el.getAttribute('type') || 'submit').toLowerCase() : type;
    if (button === 'submit' || button === 'image') { const f = formOf(el); if (f) submitForm(f, el, true); }
    else if (button === 'reset') { const f = formOf(el); if (f) f.reset(); }
    else if (t === 'select') {
      const opts = el.options;
      if (opts.length) { el.selectedIndex = (el.selectedIndex + 1) % opts.length; fire(el, 'input', InputEvent); fire(el, 'change'); }
    } else if (t !== 'input' && t !== 'textarea' && t !== 'a') {
      const label = el.closest('label');
      if (label) {
        const control = label.control;
        if (control && control !== el && !control.contains(el)) { if (isText(control) || control.localName === 'select') control.focus(); else control.click(); }
      }
    }
    return ok;
  }
  function disabledControl(el) { for (let n = el; n && n.nodeType === 1; n = n.parentNode) if (n.hasAttribute('disabled') && (n === el || n.localName === 'fieldset')) return true; return false; }
  function radios(el) { return (formOf(el) || document).querySelectorAll('input').filter(r => r.type === 'radio' && r.name === el.name && formOf(r) === formOf(el)); }
  function controlValidity(el) {
    if (el.hasAttribute('disabled') || !el.hasAttribute('required')) return true;
    const t = el.localName === 'input' ? el.type : el.localName;
    if (t === 'checkbox') return el.checked;
    if (t === 'radio') return radios(el).some(r => r.checked);
    return el.value !== '';
  }
  class HTMLInputElement extends HTMLElement {
    get value() { return H(OP.VALUE, this._id) || ''; }
    set value(v) { v = v === null || v === undefined ? '' : String(v); H(OP.VALUE, this._id, v); if (edit.el === this) { edit.caret = Array.from(v).length; showCaret(); } }
    get defaultValue() { return this.getAttribute('value') || ''; }
    set defaultValue(v) { this.setAttribute('value', v); }
    get valueAsNumber() { const n = parseFloat(this.value); return isNaN(n) ? NaN : n; }
    set valueAsNumber(n) { this.value = String(n); }
    get form() { return formOf(this); }
    // The page shows (and CSS matches) the live state through the checked attribute, so
    // the default is kept here once the state first changes.
    get defaultChecked() { return defaultChecked.has(this) ? defaultChecked.get(this) : this.hasAttribute('checked'); }
    set defaultChecked(v) { if (defaultChecked.has(this)) defaultChecked.set(this, !!v); else this.toggleAttribute('checked', !!v); }
    get selectionStart() { return edit.el === this ? edit.caret : this.value.length; }
    get selectionEnd() { return this.selectionStart; }
    setSelectionRange(a) { if (edit.el === this) { edit.caret = Math.max(0, Math.min(+a || 0, Array.from(this.value).length)); showCaret(); } }
    select() {}
    get validity() { const valid = controlValidity(this); return { valid, valueMissing: !valid }; }
    checkValidity() { return controlValidity(this); }
    reportValidity() { return controlValidity(this); }
    setCustomValidity() {}
    get labels() { return this.id ? document.querySelectorAll('label').filter(l => l.control === this) : []; }
    get checked() { return this.hasAttribute('checked'); }
    set checked(v) { if (!defaultChecked.has(this)) defaultChecked.set(this, this.hasAttribute('checked')); this.toggleAttribute('checked', !!v); }
    get disabled() { return this.hasAttribute('disabled'); }
    set disabled(v) { this.toggleAttribute('disabled', !!v); }
    get type() { return (this.getAttribute('type') || 'text').toLowerCase(); }
  }
  reflect(HTMLInputElement, ['name', 'placeholder', 'min', 'max', 'step', 'pattern', 'autocomplete']);
  for (const p of ['required', 'readOnly', 'multiple', 'autofocus']) Object.defineProperty(HTMLInputElement.prototype, p, { get() { return this.hasAttribute(p.toLowerCase()); }, set(v) { this.toggleAttribute(p.toLowerCase(), !!v); }, configurable: true });
  Object.defineProperty(HTMLInputElement.prototype, 'maxLength', { get() { return this.hasAttribute('maxlength') ? +this.getAttribute('maxlength') : -1; }, set(v) { this.setAttribute('maxlength', v); }, configurable: true });
  class HTMLTextAreaElement extends HTMLInputElement {
    get type() { return 'textarea'; }
    get defaultValue() { return this.textContent; }
    set defaultValue(v) { this.textContent = v; }
    get rows() { return +(this.getAttribute('rows') || 2); }
    get cols() { return +(this.getAttribute('cols') || 20); }
  }
  class HTMLOptionElement extends HTMLElement {
    get value() { const v = this.getAttribute('value'); return v === null ? this.text : v; }
    set value(v) { this.setAttribute('value', v); }
    get text() { return this.textContent.replace(/\s+/g, ' ').trim(); }
    set text(v) { this.textContent = v; }
    get label() { return this.getAttribute('label') || this.text; }
    get selected() { const s = this.closest('select'); return s ? s.selectedIndex === s.options.indexOf(this) : this.hasAttribute('selected'); }
    set selected(v) { const s = this.closest('select'); if (s && v) s.selectedIndex = s.options.indexOf(this); else this.toggleAttribute('selected', !!v); }
    get defaultSelected() { return this.hasAttribute('selected'); }
    get index() { const s = this.closest('select'); return s ? s.options.indexOf(this) : 0; }
    get disabled() { return this.hasAttribute('disabled'); }
    get form() { const s = this.closest('select'); return s ? s.form : null; }
  }
  class HTMLSelectElement extends HTMLElement {
    get options() { const o = this.querySelectorAll('option'); o.item = i => o[i] || null; o.namedItem = n => o.find(x => x.id === n) || null; return o; }
    get length() { return this.options.length; }
    get selectedIndex() { const o = this.options; const i = o.findIndex(x => x.hasAttribute('selected')); return i >= 0 ? i : o.length ? 0 : -1; }
    set selectedIndex(i) { const o = this.options; o.forEach((x, k) => x.toggleAttribute('selected', k === +i)); }
    get selectedOptions() { const i = this.selectedIndex; return i >= 0 ? [this.options[i]] : []; }
    get value() { const i = this.selectedIndex; return i >= 0 ? this.options[i].value : ''; }
    set value(v) { this.selectedIndex = this.options.findIndex(x => x.value === String(v)); }
    get type() { return this.hasAttribute('multiple') ? 'select-multiple' : 'select-one'; }
    get form() { return formOf(this); }
    get disabled() { return this.hasAttribute('disabled'); }
    set disabled(v) { this.toggleAttribute('disabled', !!v); }
    item(i) { return this.options[i] || null; }
    add(opt, before) { this.insertBefore(opt, typeof before === 'number' ? this.options[before] || null : before || null); }
    checkValidity() { return !this.hasAttribute('required') || this.value !== ''; }
    reportValidity() { return this.checkValidity(); }
    setCustomValidity() {}
    get validity() { const valid = this.checkValidity(); return { valid, valueMissing: !valid }; }
  }
  reflect(HTMLSelectElement, ['name']);
  class HTMLFormElement extends HTMLElement {
    get elements() {
      const out = document.querySelectorAll('input, select, textarea, button').filter(el => formOf(el) === this);
      for (const el of out) { const n = el.getAttribute('name') || el.id; if (n && isNaN(n) && !(n in out)) out[n] = el; }
      out.namedItem = n => out.find(el => el.getAttribute('name') === n || el.id === n) || null;
      return out;
    }
    get length() { return this.elements.length; }
    get action() { return resolveUrl(this.getAttribute('action') || H(OP.LOCATION) || ''); }
    set action(v) { this.setAttribute('action', v); }
    get method() { const m = (this.getAttribute('method') || 'get').toLowerCase(); return m === 'post' || m === 'dialog' ? m : 'get'; }
    set method(v) { this.setAttribute('method', v); }
    submit() { submitForm(this, null, false); }
    requestSubmit(submitter) { submitForm(this, submitter || null, true); }
    reset() {
      if (fire(this, 'reset', Event, { cancelable: true }).defaultPrevented) return;
      for (const el of this.elements) {
        if (el.localName === 'select') { const o = el.options; const i = o.findIndex(x => x.defaultSelected); el.selectedIndex = i; }
        else if (el.localName === 'input' && (el.type === 'checkbox' || el.type === 'radio')) el.checked = el.defaultChecked;
        else if (el.localName === 'textarea' || el.localName === 'input') el.value = el.defaultValue;
      }
    }
    checkValidity() { return this.elements.every(el => !el.checkValidity || el.checkValidity()); }
    reportValidity() { return this.checkValidity(); }
  }
  reflect(HTMLFormElement, ['name', 'target', 'enctype']);
  class HTMLLabelElement extends HTMLElement {
    get htmlFor() { return this.getAttribute('for') || ''; }
    set htmlFor(v) { this.setAttribute('for', v); }
    get control() {
      const f = this.getAttribute('for');
      if (f) { const c = document.getElementById(f); return c && /^(input|select|textarea|button)$/.test(c.localName) ? c : null; }
      return this.querySelector('input, select, textarea, button');
    }
    get form() { const c = this.control; return c ? c.form : null; }
  }
  class FormData {
    constructor(form, submitter) { this._l = form ? formEntries(form, submitter || null) : []; }
    append(k, v) { this._l.push([String(k), String(v)]); }
    set(k, v) { this.delete(k); this.append(k, v); }
    delete(k) { this._l = this._l.filter(p => p[0] !== String(k)); }
    get(k) { const p = this._l.find(p => p[0] === String(k)); return p ? p[1] : null; }
    getAll(k) { return this._l.filter(p => p[0] === String(k)).map(p => p[1]); }
    has(k) { return this._l.some(p => p[0] === String(k)); }
    entries() { return this._l.map(p => p.slice())[Symbol.iterator](); }
    keys() { return this._l.map(p => p[0])[Symbol.iterator](); }
    values() { return this._l.map(p => p[1])[Symbol.iterator](); }
    forEach(fn, self) { for (const [k, v] of this._l) fn.call(self, v, k, this); }
    [Symbol.iterator]() { return this.entries(); }
  }
  class HTMLButtonElement extends HTMLElement {
    get form() { return formOf(this); }
    checkValidity() { return true; }
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
    set width(v) { this.setAttribute('width', Math.max(0, Math.floor(+v) || 0)); this._resized(); }
    get height() { return +(this.getAttribute('height') || 150); }
    set height(v) { this.setAttribute('height', Math.max(0, Math.floor(+v) || 0)); this._resized(); }
    // Setting either size, even to the same value, clears the bitmap and the context's state.
    _resized() {
      if (this._ctx) this._ctx._resize(this.width, this.height);
      if (this._gl) this._gl._resize(this.width, this.height);
      if (this._gpu) this._gpu._resize(this.width, this.height);
    }
    getContext(kind, attrs) {
      kind = String(kind).toLowerCase();
      if (kind === '2d') return this._gl || this._gpu ? null : this._ctx || (this._ctx = new CanvasRenderingContext2D(this));
      if (kind === 'webgpu') return this._ctx || this._gl ? null : this._gpu || (this._gpu = new GPUCanvasContext(this));
      if (kind !== 'webgl' && kind !== 'experimental-webgl' && kind !== 'webgl2') return null;
      if (this._ctx || this._gpu) return null;
      const v2 = kind === 'webgl2';
      if (this._gl) return this._gl._v2 === v2 || !v2 ? this._gl : null;
      return (this._gl = v2 ? new WebGL2RenderingContext(this, attrs) : new WebGLRenderingContext(this, attrs));
    }
    toDataURL() { return 'data:,'; }
  }

  // ---- Canvas 2D ----
  // Calls are queued as commands (fields split by \x1f, commands by \x1e) and sent in
  // batches; the browser draws them with tiny-skia and Geist. State lives on both sides:
  // here for reading back, there for drawing.
  const FS = '\x1f';
  const dirtyContexts = new Set();
  function flushCanvases() {
    for (const c of dirtyContexts) c._flush();
    dirtyContexts.clear();
  }
  // Whole numbers skip the float rounding and printing, which are slow without a float unit.
  const num = v => { v = +v; return (v | 0) === v ? '' + v : Number.isFinite(v) ? String(Math.round(v * 1000) / 1000) : null; };
  class CanvasGradient {
    constructor(kind, args) { this._kind = kind; this._args = args; this._stops = []; this._v = 0; }
    addColorStop(offset, color) { this._stops.push([+offset, String(color)]); this._v++; }
    _encode() { return [this._kind, ...this._args.map(num), this._stops.length, ...this._stops.flat().map(String)]; }
  }
  class CanvasPattern {}
  class CanvasRenderingContext2D {
    constructor(canvas) {
      this.canvas = canvas;
      this._q = [];
      this._s = { fillStyle: '#000000', strokeStyle: '#000000', lineWidth: 1, globalAlpha: 1, lineCap: 'butt', lineJoin: 'miter',
        font: '10px sans-serif', textAlign: 'start', textBaseline: 'alphabetic', globalCompositeOperation: 'source-over',
        miterLimit: 10, shadowBlur: 0, shadowColor: 'rgba(0, 0, 0, 0)', shadowOffsetX: 0, shadowOffsetY: 0, imageSmoothingEnabled: true,
        lineDashOffset: 0 };
      this._stack = [];
      this._dash = [];
    }
    _c(...a) {
      for (let i = 1; i < a.length; i++) if (a[i] === null) return; // NaN or Infinity: the call is ignored
      this._q.push(a.join(FS));
      dirtyContexts.add(this);
      if (this._q.length >= 2048) this._flush();
    }
    _flush() { if (this._q.length) { H(OP.CANVAS, this.canvas._id, this._q.join('\x1e')); this._q = []; } }
    _style(which, v) {
      if (v instanceof CanvasGradient) { this._s[which] = v; this._s[which + 'V'] = v._v; this._c(which === 'fillStyle' ? 'fs' : 'ss', ...v._encode()); return; }
      if (typeof v !== 'string') return;
      this._s[which] = v;
      this._c(which === 'fillStyle' ? 'fs' : 'ss', 'c', v);
    }
    // A gradient can gain stops after it is assigned: send it again before drawing with it.
    _sync(which) {
      const g = this._s[which];
      if (g instanceof CanvasGradient && this._s[which + 'V'] !== g._v) this._style(which, g);
    }
    _resize(w, h) {
      this._s = new CanvasRenderingContext2D(this.canvas)._s; this._stack = []; this._dash = [];
      this._c('sz', w, h);
    }
    get fillStyle() { return this._s.fillStyle; }
    set fillStyle(v) { this._style('fillStyle', v); }
    get strokeStyle() { return this._s.strokeStyle; }
    set strokeStyle(v) { this._style('strokeStyle', v); }
    get lineWidth() { return this._s.lineWidth; }
    set lineWidth(v) { v = +v; if (v > 0 && Number.isFinite(v)) { this._s.lineWidth = v; this._c('lw', num(v)); } }
    get globalAlpha() { return this._s.globalAlpha; }
    set globalAlpha(v) { v = +v; if (v >= 0 && v <= 1) { this._s.globalAlpha = v; this._c('ga', num(v)); } }
    get lineCap() { return this._s.lineCap; }
    set lineCap(v) { if (['butt', 'round', 'square'].includes(v)) { this._s.lineCap = v; this._c('lc', v); } }
    get lineJoin() { return this._s.lineJoin; }
    set lineJoin(v) { if (['miter', 'round', 'bevel'].includes(v)) { this._s.lineJoin = v; this._c('lj', v); } }
    get font() { return this._s.font; }
    set font(v) { this._s.font = String(v); this._c('fo', this._s.font); }
    get textAlign() { return this._s.textAlign; }
    set textAlign(v) { if (['start', 'end', 'left', 'right', 'center'].includes(v)) { this._s.textAlign = v; this._c('ta', v); } }
    get textBaseline() { return this._s.textBaseline; }
    set textBaseline(v) { if (['top', 'hanging', 'middle', 'alphabetic', 'ideographic', 'bottom'].includes(v)) { this._s.textBaseline = v; this._c('tb', v); } }
    get globalCompositeOperation() { return this._s.globalCompositeOperation; }
    set globalCompositeOperation(v) { this._s.globalCompositeOperation = String(v); this._c('co', this._s.globalCompositeOperation); }
    save() { this._stack.push(Object.assign({}, this._s)); this._c('sv'); }
    restore() { const s = this._stack.pop(); if (s) { this._s = s; this._c('rs'); } }
    reset() { this._s = new CanvasRenderingContext2D(this.canvas)._s; this._stack = []; this._c('re0'); }
    setTransform(a, b, c, d, e, f) {
      if (typeof a === 'object' && a !== null) ({ a, b, c, d, e, f } = a);
      if (a === undefined) [a, b, c, d, e, f] = [1, 0, 0, 1, 0, 0];
      this._c('st', num(a), num(b), num(c), num(d), num(e), num(f));
    }
    resetTransform() { this.setTransform(1, 0, 0, 1, 0, 0); }
    transform(a, b, c, d, e, f) { this._c('tf', num(a), num(b), num(c), num(d), num(e), num(f)); }
    translate(x, y) { this._c('tf', 1, 0, 0, 1, num(x), num(y)); }
    scale(x, y) { this._c('tf', num(x), 0, 0, num(y), 0, 0); }
    rotate(a) { const c = Math.cos(a), s = Math.sin(a); this._c('tf', num(c), num(s), num(-s), num(c), 0, 0); }
    beginPath() { this._c('bp'); }
    closePath() { this._c('cp'); }
    moveTo(x, y) { this._c('mt', num(x), num(y)); }
    lineTo(x, y) { this._c('lt', num(x), num(y)); }
    quadraticCurveTo(cx, cy, x, y) { this._c('qc', num(cx), num(cy), num(x), num(y)); }
    bezierCurveTo(a, b, c, d, x, y) { this._c('bc', num(a), num(b), num(c), num(d), num(x), num(y)); }
    arc(x, y, r, a0, a1, ccw) { if (r < 0) throw domError('Negative radius'); this._c('el', num(x), num(y), num(r), num(r), 0, num(a0), num(a1), ccw ? 1 : 0); }
    ellipse(x, y, rx, ry, rot, a0, a1, ccw) { if (rx < 0 || ry < 0) throw domError('Negative radius'); this._c('el', num(x), num(y), num(rx), num(ry), num(rot), num(a0), num(a1), ccw ? 1 : 0); }
    arcTo(x1, y1, x2, y2, r) { this._c('at', num(x1), num(y1), num(x2), num(y2), num(r)); }
    rect(x, y, w, h) { this._c('rc', num(x), num(y), num(w), num(h)); }
    roundRect(x, y, w, h, r) { const v = Array.isArray(r) ? r[0] : r; this._c('rr', num(x), num(y), num(w), num(h), num(v || 0)); }
    fill(a, b) { this._sync('fillStyle'); const rule = typeof a === 'string' ? a : b; this._c('fi', rule === 'evenodd' ? 'evenodd' : 'nonzero'); }
    stroke() { this._sync('strokeStyle'); this._c('sk'); }
    clip() { /* not yet: drawing is not clipped */ }
    fillRect(x, y, w, h) { this._sync('fillStyle'); this._c('fr', num(x), num(y), num(w), num(h)); }
    strokeRect(x, y, w, h) { this._sync('strokeStyle'); this._c('sr', num(x), num(y), num(w), num(h)); }
    clearRect(x, y, w, h) { this._c('cr', num(x), num(y), num(w), num(h)); }
    fillText(t, x, y) { this._sync('fillStyle'); this._c('ft', String(t).replace(/[\x1e\x1f]/g, ' '), num(x), num(y)); }
    strokeText(t, x, y) { this.fillText(t, x, y); }
    measureText(t) {
      const width = +H(OP.MEASURE, this._s.font, String(t));
      const size = (/(\d+(?:\.\d+)?)px/.exec(this._s.font) || [0, 10])[1] * 1;
      return { width, actualBoundingBoxLeft: 0, actualBoundingBoxRight: width, actualBoundingBoxAscent: size * 0.75,
        actualBoundingBoxDescent: size * 0.25, fontBoundingBoxAscent: size * 0.9, fontBoundingBoxDescent: size * 0.25 };
    }
    createLinearGradient(x0, y0, x1, y1) { return new CanvasGradient('l', [x0, y0, x1, y1]); }
    createRadialGradient(x0, y0, r0, x1, y1, r1) { return new CanvasGradient('r', [x0, y0, r0, x1, y1, r1]); }
    createPattern() { return null; }
    setLineDash(d) { this._dash = Array.isArray(d) ? d.slice() : []; }
    getLineDash() { return this._dash.slice(); }
    isPointInPath() { return false; }
    // Draws a picture (an <img> or another canvas) as drawImage(src, dx, dy[, dw, dh]) or
    // drawImage(src, sx, sy, sw, sh, dx, dy, dw, dh). A canvas source shows what it has drawn so far.
    drawImage(src, ...a) {
      let name, w, h;
      if (src && src.tagName === 'CANVAS') {
        // The copy must see the source as it is now: what this canvas queued before goes
        // first, then the source's own drawing, then the copy.
        if (src._ctx && src._ctx._q.length) { this._flush(); src._ctx._flush(); }
        name = 'canvas:' + src._id; w = src.width; h = src.height;
      } else if (src && src.tagName === 'IMG') {
        name = src.getAttribute('src') || ''; w = src.naturalWidth || 0; h = src.naturalHeight || 0;
      } else return;
      let sx = 0, sy = 0, sw = w, sh = h, dx, dy, dw, dh;
      if (a.length >= 8) [sx, sy, sw, sh, dx, dy, dw, dh] = a;
      else if (a.length >= 4) { [dx, dy, dw, dh] = a; sw = w; sh = h; }
      else { [dx, dy] = a; dw = w; dh = h; }
      if (a.length < 8 && !(w > 0 && h > 0)) { sw = sh = -1; }
      this._c('di', name.replace(/[\x1e\x1f]/g, ' '), num(sx), num(sy), num(sw), num(sh), num(dx), num(dy), num(dw === undefined ? sw : dw), num(dh === undefined ? sh : dh));
    }
    getImageData(x, y, w, h) { return { width: w, height: h, data: new Uint8ClampedArray(Math.max(0, w * h * 4)) }; }
    createImageData(w, h) { return { width: w, height: h, data: new Uint8ClampedArray(Math.max(0, w * h * 4)) }; }
    putImageData() {}
  }
  const ELEMENT_CLASSES = { A: HTMLAnchorElement, IMG: HTMLImageElement, INPUT: HTMLInputElement, BUTTON: HTMLButtonElement,
    TEMPLATE: HTMLTemplateElement, CANVAS: HTMLCanvasElement, TEXTAREA: HTMLTextAreaElement, SELECT: HTMLSelectElement,
    OPTION: HTMLOptionElement, FORM: HTMLFormElement, LABEL: HTMLLabelElement };

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
    constructor() { super(0); this._active = null; this.readyState = 'loading'; this.pointerLockElement = null; }
    get nodeType() { return 9; }
    elementFromPoint(x, y) { const id = H(OP.HIT, String(Math.floor(x)), String(Math.floor(y + window.scrollY))); return id ? wrap(id) : null; }
    elementsFromPoint(x, y) { const e = this.elementFromPoint(x, y); return e ? chain(e) : []; }
    exitPointerLock() { if (mouse.lock) { mouse.lock = null; this.pointerLockElement = null; this.dispatchEvent(new Event('pointerlockchange')); } }
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
    get cookie() { const v = H(OP.COOKIE); return v === null ? '' : v; }
    set cookie(v) { H(OP.COOKIE, String(v)); }
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

  // ---- WebGL ----
  // Calls are queued as commands (fields split by \x1f, commands by \x1e) with their bulk
  // data gathered into one ArrayBuffer, and sent in batches like Canvas 2D. Calls that
  // need an answer flush the batch first and ask the browser (OP.GL_QUERY).
  const GL_CONSTANTS = {
    DEPTH_BUFFER_BIT: 0x100, STENCIL_BUFFER_BIT: 0x400, COLOR_BUFFER_BIT: 0x4000,
    POINTS: 0, LINES: 1, LINE_LOOP: 2, LINE_STRIP: 3, TRIANGLES: 4, TRIANGLE_STRIP: 5, TRIANGLE_FAN: 6,
    ZERO: 0, ONE: 1, SRC_COLOR: 0x300, ONE_MINUS_SRC_COLOR: 0x301, SRC_ALPHA: 0x302, ONE_MINUS_SRC_ALPHA: 0x303,
    DST_ALPHA: 0x304, ONE_MINUS_DST_ALPHA: 0x305, DST_COLOR: 0x306, ONE_MINUS_DST_COLOR: 0x307, SRC_ALPHA_SATURATE: 0x308,
    FUNC_ADD: 0x8006, BLEND_EQUATION: 0x8009, BLEND_EQUATION_RGB: 0x8009, BLEND_EQUATION_ALPHA: 0x883D,
    FUNC_SUBTRACT: 0x800A, FUNC_REVERSE_SUBTRACT: 0x800B, MIN: 0x8007, MAX: 0x8008,
    BLEND_DST_RGB: 0x80C8, BLEND_SRC_RGB: 0x80C9, BLEND_DST_ALPHA: 0x80CA, BLEND_SRC_ALPHA: 0x80CB,
    CONSTANT_COLOR: 0x8001, ONE_MINUS_CONSTANT_COLOR: 0x8002, CONSTANT_ALPHA: 0x8003, ONE_MINUS_CONSTANT_ALPHA: 0x8004, BLEND_COLOR: 0x8005,
    ARRAY_BUFFER: 0x8892, ELEMENT_ARRAY_BUFFER: 0x8893, ARRAY_BUFFER_BINDING: 0x8894, ELEMENT_ARRAY_BUFFER_BINDING: 0x8895,
    STREAM_DRAW: 0x88E0, STATIC_DRAW: 0x88E4, DYNAMIC_DRAW: 0x88E8, BUFFER_SIZE: 0x8764, BUFFER_USAGE: 0x8765,
    CURRENT_VERTEX_ATTRIB: 0x8626, FRONT: 0x404, BACK: 0x405, FRONT_AND_BACK: 0x408,
    CULL_FACE: 0xB44, BLEND: 0xBE2, DITHER: 0xBD0, STENCIL_TEST: 0xB90, DEPTH_TEST: 0xB71, SCISSOR_TEST: 0xC11,
    POLYGON_OFFSET_FILL: 0x8037, SAMPLE_ALPHA_TO_COVERAGE: 0x809E, SAMPLE_COVERAGE: 0x80A0, RASTERIZER_DISCARD: 0x8C89,
    NO_ERROR: 0, INVALID_ENUM: 0x500, INVALID_VALUE: 0x501, INVALID_OPERATION: 0x502, OUT_OF_MEMORY: 0x505, INVALID_FRAMEBUFFER_OPERATION: 0x506,
    CW: 0x900, CCW: 0x901, LINE_WIDTH: 0xB21, ALIASED_POINT_SIZE_RANGE: 0x846D, ALIASED_LINE_WIDTH_RANGE: 0x846E,
    CULL_FACE_MODE: 0xB45, FRONT_FACE: 0xB46, DEPTH_RANGE: 0xB70, DEPTH_WRITEMASK: 0xB72, DEPTH_CLEAR_VALUE: 0xB73, DEPTH_FUNC: 0xB74,
    STENCIL_CLEAR_VALUE: 0xB91, STENCIL_FUNC: 0xB92, STENCIL_FAIL: 0xB94, STENCIL_PASS_DEPTH_FAIL: 0xB95, STENCIL_PASS_DEPTH_PASS: 0xB96,
    STENCIL_REF: 0xB97, STENCIL_VALUE_MASK: 0xB93, STENCIL_WRITEMASK: 0xB98, STENCIL_BACK_FUNC: 0x8800, STENCIL_BACK_FAIL: 0x8801,
    STENCIL_BACK_PASS_DEPTH_FAIL: 0x8802, STENCIL_BACK_PASS_DEPTH_PASS: 0x8803, STENCIL_BACK_REF: 0x8CA3, STENCIL_BACK_VALUE_MASK: 0x8CA4,
    STENCIL_BACK_WRITEMASK: 0x8CA5, VIEWPORT: 0xBA2, SCISSOR_BOX: 0xC10, COLOR_CLEAR_VALUE: 0xC22, COLOR_WRITEMASK: 0xC23,
    UNPACK_ALIGNMENT: 0xCF5, PACK_ALIGNMENT: 0xD05, MAX_TEXTURE_SIZE: 0xD33, MAX_VIEWPORT_DIMS: 0xD3A, SUBPIXEL_BITS: 0xD50,
    RED_BITS: 0xD52, GREEN_BITS: 0xD53, BLUE_BITS: 0xD54, ALPHA_BITS: 0xD55, DEPTH_BITS: 0xD56, STENCIL_BITS: 0xD57,
    POLYGON_OFFSET_UNITS: 0x2A00, POLYGON_OFFSET_FACTOR: 0x8038, TEXTURE_BINDING_2D: 0x8069, SAMPLE_BUFFERS: 0x80A8, SAMPLES: 0x80A9,
    SAMPLE_COVERAGE_VALUE: 0x80AA, SAMPLE_COVERAGE_INVERT: 0x80AB, COMPRESSED_TEXTURE_FORMATS: 0x86A3,
    DONT_CARE: 0x1100, FASTEST: 0x1101, NICEST: 0x1102, GENERATE_MIPMAP_HINT: 0x8192,
    BYTE: 0x1400, UNSIGNED_BYTE: 0x1401, SHORT: 0x1402, UNSIGNED_SHORT: 0x1403, INT: 0x1404, UNSIGNED_INT: 0x1405, FLOAT: 0x1406,
    DEPTH_COMPONENT: 0x1902, ALPHA: 0x1906, RGB: 0x1907, RGBA: 0x1908, LUMINANCE: 0x1909, LUMINANCE_ALPHA: 0x190A,
    UNSIGNED_SHORT_4_4_4_4: 0x8033, UNSIGNED_SHORT_5_5_5_1: 0x8034, UNSIGNED_SHORT_5_6_5: 0x8363,
    FRAGMENT_SHADER: 0x8B30, VERTEX_SHADER: 0x8B31, MAX_VERTEX_ATTRIBS: 0x8869, MAX_VERTEX_UNIFORM_VECTORS: 0x8DFB,
    MAX_VARYING_VECTORS: 0x8DFC, MAX_COMBINED_TEXTURE_IMAGE_UNITS: 0x8B4D, MAX_VERTEX_TEXTURE_IMAGE_UNITS: 0x8B4C,
    MAX_TEXTURE_IMAGE_UNITS: 0x8872, MAX_FRAGMENT_UNIFORM_VECTORS: 0x8DFD, SHADER_TYPE: 0x8B4F, DELETE_STATUS: 0x8B80,
    LINK_STATUS: 0x8B82, VALIDATE_STATUS: 0x8B83, ATTACHED_SHADERS: 0x8B85, ACTIVE_UNIFORMS: 0x8B86, ACTIVE_ATTRIBUTES: 0x8B89,
    SHADING_LANGUAGE_VERSION: 0x8B8C, CURRENT_PROGRAM: 0x8B8D, NEVER: 0x200, LESS: 0x201, EQUAL: 0x202, LEQUAL: 0x203,
    GREATER: 0x204, NOTEQUAL: 0x205, GEQUAL: 0x206, ALWAYS: 0x207, KEEP: 0x1E00, REPLACE: 0x1E01, INCR: 0x1E02, DECR: 0x1E03,
    INVERT: 0x150A, INCR_WRAP: 0x8507, DECR_WRAP: 0x8508, VENDOR: 0x1F00, RENDERER: 0x1F01, VERSION: 0x1F02,
    NEAREST: 0x2600, LINEAR: 0x2601, NEAREST_MIPMAP_NEAREST: 0x2700, LINEAR_MIPMAP_NEAREST: 0x2701, NEAREST_MIPMAP_LINEAR: 0x2702,
    LINEAR_MIPMAP_LINEAR: 0x2703, TEXTURE_MAG_FILTER: 0x2800, TEXTURE_MIN_FILTER: 0x2801, TEXTURE_WRAP_S: 0x2802, TEXTURE_WRAP_T: 0x2803,
    TEXTURE_2D: 0xDE1, TEXTURE: 0x1702, TEXTURE_CUBE_MAP: 0x8513, TEXTURE_BINDING_CUBE_MAP: 0x8514,
    TEXTURE_CUBE_MAP_POSITIVE_X: 0x8515, TEXTURE_CUBE_MAP_NEGATIVE_X: 0x8516, TEXTURE_CUBE_MAP_POSITIVE_Y: 0x8517,
    TEXTURE_CUBE_MAP_NEGATIVE_Y: 0x8518, TEXTURE_CUBE_MAP_POSITIVE_Z: 0x8519, TEXTURE_CUBE_MAP_NEGATIVE_Z: 0x851A,
    MAX_CUBE_MAP_TEXTURE_SIZE: 0x851C, ACTIVE_TEXTURE: 0x84E0, REPEAT: 0x2901, CLAMP_TO_EDGE: 0x812F, MIRRORED_REPEAT: 0x8370,
    FLOAT_VEC2: 0x8B50, FLOAT_VEC3: 0x8B51, FLOAT_VEC4: 0x8B52, INT_VEC2: 0x8B53, INT_VEC3: 0x8B54, INT_VEC4: 0x8B55, BOOL: 0x8B56,
    BOOL_VEC2: 0x8B57, BOOL_VEC3: 0x8B58, BOOL_VEC4: 0x8B59, FLOAT_MAT2: 0x8B5A, FLOAT_MAT3: 0x8B5B, FLOAT_MAT4: 0x8B5C,
    SAMPLER_2D: 0x8B5E, SAMPLER_CUBE: 0x8B60, VERTEX_ATTRIB_ARRAY_ENABLED: 0x8622, VERTEX_ATTRIB_ARRAY_SIZE: 0x8623,
    VERTEX_ATTRIB_ARRAY_STRIDE: 0x8624, VERTEX_ATTRIB_ARRAY_TYPE: 0x8625, VERTEX_ATTRIB_ARRAY_NORMALIZED: 0x886A,
    VERTEX_ATTRIB_ARRAY_POINTER: 0x8645, VERTEX_ATTRIB_ARRAY_BUFFER_BINDING: 0x889F, IMPLEMENTATION_COLOR_READ_TYPE: 0x8B9A,
    IMPLEMENTATION_COLOR_READ_FORMAT: 0x8B9B, COMPILE_STATUS: 0x8B81, LOW_FLOAT: 0x8DF0, MEDIUM_FLOAT: 0x8DF1, HIGH_FLOAT: 0x8DF2,
    LOW_INT: 0x8DF3, MEDIUM_INT: 0x8DF4, HIGH_INT: 0x8DF5, FRAMEBUFFER: 0x8D40, RENDERBUFFER: 0x8D41, RGBA4: 0x8056, RGB5_A1: 0x8057,
    RGB565: 0x8D62, DEPTH_COMPONENT16: 0x81A5, STENCIL_INDEX8: 0x8D48, DEPTH_STENCIL: 0x84F9, RENDERBUFFER_WIDTH: 0x8D42,
    RENDERBUFFER_HEIGHT: 0x8D43, RENDERBUFFER_INTERNAL_FORMAT: 0x8D44, COLOR_ATTACHMENT0: 0x8CE0, DEPTH_ATTACHMENT: 0x8D00,
    STENCIL_ATTACHMENT: 0x8D20, DEPTH_STENCIL_ATTACHMENT: 0x821A, NONE: 0, FRAMEBUFFER_COMPLETE: 0x8CD5,
    FRAMEBUFFER_INCOMPLETE_ATTACHMENT: 0x8CD6, FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT: 0x8CD7, FRAMEBUFFER_UNSUPPORTED: 0x8CDD,
    FRAMEBUFFER_BINDING: 0x8CA6, RENDERBUFFER_BINDING: 0x8CA7, MAX_RENDERBUFFER_SIZE: 0x84E8, UNPACK_FLIP_Y_WEBGL: 0x9240,
    UNPACK_PREMULTIPLY_ALPHA_WEBGL: 0x9241, CONTEXT_LOST_WEBGL: 0x9242, UNPACK_COLORSPACE_CONVERSION_WEBGL: 0x9243, BROWSER_DEFAULT_WEBGL: 0x9244,
  };
  const GL2_CONSTANTS = {
    RED: 0x1903, RG: 0x8227, R8: 0x8229, RG8: 0x822B, RGB8: 0x8051, RGBA8: 0x8058, SRGB8: 0x8C41, SRGB8_ALPHA8: 0x8C43,
    R16F: 0x822D, RG16F: 0x822F, RGB16F: 0x881B, RGBA16F: 0x881A, R32F: 0x822E, RG32F: 0x8230, RGB32F: 0x8815, RGBA32F: 0x8814,
    HALF_FLOAT: 0x140B, RED_INTEGER: 0x8D94, RG_INTEGER: 0x8228, RGB_INTEGER: 0x8D98, RGBA_INTEGER: 0x8D99,
    DEPTH_COMPONENT24: 0x81A6, DEPTH_COMPONENT32F: 0x8CAC, DEPTH24_STENCIL8: 0x88F0, UNSIGNED_INT_24_8: 0x84FA,
    TEXTURE_3D: 0x806F, TEXTURE_2D_ARRAY: 0x8C1A, TEXTURE_WRAP_R: 0x8072, TEXTURE_MIN_LOD: 0x813A, TEXTURE_MAX_LOD: 0x813B,
    TEXTURE_BASE_LEVEL: 0x813C, TEXTURE_MAX_LEVEL: 0x813D, TEXTURE_COMPARE_MODE: 0x884C, TEXTURE_COMPARE_FUNC: 0x884D,
    COMPARE_REF_TO_TEXTURE: 0x884E, MAX_3D_TEXTURE_SIZE: 0x8073, MAX_ARRAY_TEXTURE_LAYERS: 0x88FF, MAX_SAMPLES: 0x8D57,
    MAX_DRAW_BUFFERS: 0x8824, MAX_COLOR_ATTACHMENTS: 0x8CDF, MAX_UNIFORM_BUFFER_BINDINGS: 0x8A2F, MAX_UNIFORM_BLOCK_SIZE: 0x8A30,
    MAX_VERTEX_UNIFORM_COMPONENTS: 0x8B4A, MAX_FRAGMENT_UNIFORM_COMPONENTS: 0x8B49, MAX_ELEMENT_INDEX: 0x8D6B,
    UNIFORM_BUFFER: 0x8A11, COPY_READ_BUFFER: 0x8F36, COPY_WRITE_BUFFER: 0x8F37, PIXEL_PACK_BUFFER: 0x88EB, PIXEL_UNPACK_BUFFER: 0x88EC,
    TRANSFORM_FEEDBACK_BUFFER: 0x8C8E, VERTEX_ARRAY_BINDING: 0x85B5, READ_FRAMEBUFFER: 0x8CA8, DRAW_FRAMEBUFFER: 0x8CA9,
    UNSIGNED_INT_VEC2: 0x8DC6, UNSIGNED_INT_VEC3: 0x8DC7, UNSIGNED_INT_VEC4: 0x8DC8, SAMPLER_3D: 0x8B5F, SAMPLER_2D_SHADOW: 0x8B62,
    SAMPLER_2D_ARRAY: 0x8DC1, STATIC_READ: 0x88E5, DYNAMIC_READ: 0x88E9, STREAM_READ: 0x88E1, STATIC_COPY: 0x88E6,
    DYNAMIC_COPY: 0x88EA, STREAM_COPY: 0x88E2, UNPACK_ROW_LENGTH: 0xCF2, UNPACK_SKIP_ROWS: 0xCF3, UNPACK_SKIP_PIXELS: 0xCF4,
    PACK_ROW_LENGTH: 0xD02, UNPACK_IMAGE_HEIGHT: 0x806E, UNPACK_SKIP_IMAGES: 0x806D, COLOR_ATTACHMENT1: 0x8CE1, DRAW_BUFFER0: 0x8825,
    SYNC_GPU_COMMANDS_COMPLETE: 0x9117, ALREADY_SIGNALED: 0x911A, TIMEOUT_EXPIRED: 0x911B, CONDITION_SATISFIED: 0x911C,
    WAIT_FAILED: 0x911D, SYNC_FLUSH_COMMANDS_BIT: 0x1, TIMEOUT_IGNORED: -1, ANY_SAMPLES_PASSED: 0x8C2F,
    QUERY_RESULT: 0x8866, QUERY_RESULT_AVAILABLE: 0x8867, INVALID_INDEX: 0xFFFFFFFF, VERTEX_ATTRIB_ARRAY_DIVISOR: 0x88FE,
    VERTEX_ATTRIB_ARRAY_INTEGER: 0x88FD, UNIFORM_BLOCK_INDEX: 0x8A3A, UNIFORM_OFFSET: 0x8A3B, ACTIVE_UNIFORM_BLOCKS: 0x8A36,
    UNIFORM_BLOCK_DATA_SIZE: 0x8A40, MAX_SERVER_WAIT_TIMEOUT: 0x9111, MAX_TEXTURE_LOD_BIAS: 0x84FD,
    R8UI: 0x8232, R16UI: 0x8234, R32UI: 0x8236, RG8UI: 0x8238, RG16UI: 0x823A, RG32UI: 0x823C, RGB8UI: 0x8D7D,
    RGB16UI: 0x8D77, RGB32UI: 0x8D71, RGBA8UI: 0x8D7C, RGBA16UI: 0x8D76, RGBA32UI: 0x8D70, R8I: 0x8231, R16I: 0x8233,
    R32I: 0x8235, RG8I: 0x8237, RG16I: 0x8239, RG32I: 0x823B, RGB8I: 0x8D8F, RGB16I: 0x8D89, RGB32I: 0x8D83,
    RGBA8I: 0x8D8E, RGBA16I: 0x8D88, RGBA32I: 0x8D82, RGB10_A2: 0x8059, RGB10_A2UI: 0x906F, R11F_G11F_B10F: 0x8C3A,
    RGB9_E5: 0x8C3D, RGBA4: 0x8056, RGB5_A1: 0x8057, R8_SNORM: 0x8F94, RG8_SNORM: 0x8F95, RGB8_SNORM: 0x8F96, RGBA8_SNORM: 0x8F97,
    INT_SAMPLER_2D: 0x8DCA, INT_SAMPLER_3D: 0x8DCB, INT_SAMPLER_CUBE: 0x8DCC, INT_SAMPLER_2D_ARRAY: 0x8DCF,
    UNSIGNED_INT_SAMPLER_2D: 0x8DD2, UNSIGNED_INT_SAMPLER_3D: 0x8DD3, UNSIGNED_INT_SAMPLER_CUBE: 0x8DD4,
    UNSIGNED_INT_SAMPLER_2D_ARRAY: 0x8DD7, SAMPLER_CUBE_SHADOW: 0x8DC5, SAMPLER_2D_ARRAY_SHADOW: 0x8DC4,
    FLOAT_MAT2x3: 0x8B65, FLOAT_MAT2x4: 0x8B66, FLOAT_MAT3x2: 0x8B67, FLOAT_MAT3x4: 0x8B68, FLOAT_MAT4x2: 0x8B69, FLOAT_MAT4x3: 0x8B6A,
  };
  const bytesOf = v => v instanceof ArrayBuffer ? new Uint8Array(v) : ArrayBuffer.isView(v) ? new Uint8Array(v.buffer, v.byteOffset, v.byteLength) : null;
  let glNextId = 1;
  class WebGLObject { constructor() { this._id = glNextId++; } }
  class WebGLBuffer extends WebGLObject {}
  class WebGLTexture extends WebGLObject {}
  class WebGLProgram extends WebGLObject {}
  class WebGLShader extends WebGLObject {}
  class WebGLFramebuffer extends WebGLObject {}
  class WebGLRenderbuffer extends WebGLObject {}
  class WebGLVertexArrayObject extends WebGLObject {}
  class WebGLQuery extends WebGLObject {}
  class WebGLSampler extends WebGLObject {}
  class WebGLSync extends WebGLObject {}
  class WebGLTransformFeedback extends WebGLObject {}
  class WebGLUniformLocation { constructor(p, loc) { this._p = p; this._loc = loc; } }
  class WebGLActiveInfo { constructor(name, type, size) { this.name = name; this.type = type; this.size = size; } }
  class WebGLShaderPrecisionFormat { constructor(r, p) { this.rangeMin = r; this.rangeMax = r; this.precision = p; } }
  const numStr = v => { v = +v; return Number.isFinite(v) ? String(v) : '0'; };

  class WebGLRenderingContext {
    constructor(canvas, attrs, v2) {
      this.canvas = canvas;
      this._v2 = !!v2;
      this._attrs = Object.assign({ alpha: true, depth: true, stencil: false, antialias: true, premultipliedAlpha: true,
        preserveDrawingBuffer: false, powerPreference: 'default', failIfMajorPerformanceCaveat: false, desynchronized: false }, attrs || {});
      this._q = []; this._bin = []; this._binLen = 0;
      this._s = { program: null, array: null, vao: null, unit: 0, tex: [], clear: [0, 0, 0, 0], depthClear: 1, viewport: [0, 0, canvas.width, canvas.height],
        scissor: [0, 0, canvas.width, canvas.height], enabled: {}, depthFunc: 0x201, depthMask: true, colorMask: [true, true, true, true],
        blend: [1, 0, 1, 0], blendEq: [0x8006, 0x8006], cullFace: 0x405, frontFace: 0x901, flipY: false, premul: false, align: 4,
        lineWidth: 1, elem: new Map(), fb: null, readFb: null, rb: null };
      this._progInfo = new Map();
      H(OP.GL_QUERY, canvas._id, 'init', canvas.width, canvas.height, this._attrs.alpha === false ? 0 : 1);
    }
    get drawingBufferWidth() { return this.canvas.width; }
    get drawingBufferHeight() { return this.canvas.height; }
    get drawingBufferColorSpace() { return 'srgb'; }
    set drawingBufferColorSpace(v) {}
    _c(...a) { this._q.push(a.join(FS)); dirtyContexts.add(this); if (this._q.length >= 4096) this._flush(); }
    _data(v) {
      const b = bytesOf(v);
      if (!b) return [0, 0];
      const off = this._binLen;
      this._bin.push(b.slice());
      this._binLen += b.byteLength;
      return [off, b.byteLength];
    }
    _flush() {
      if (!this._q.length) return;
      const all = new Uint8Array(this._binLen);
      let at = 0;
      for (const b of this._bin) { all.set(b, at); at += b.byteLength; }
      const q = this._q.join('\x1e');
      this._q = []; this._bin = []; this._binLen = 0;
      H(OP.GL, this.canvas._id, q, all.buffer);
    }
    _ask(what, ...a) { this._flush(); return H(OP.GL_QUERY, this.canvas._id, what, ...a.map(String)); }
    _resize(w, h) { this._c('rz', w, h); this._s.viewport = [0, 0, w, h]; }
    getContextAttributes() { return Object.assign({}, this._attrs); }
    isContextLost() { return false; }
    getSupportedExtensions() { return ['OES_element_index_uint', 'OES_standard_derivatives', 'OES_vertex_array_object', 'ANGLE_instanced_arrays', 'OES_texture_float',
      'OES_texture_half_float', 'EXT_blend_minmax', 'WEBGL_lose_context', 'EXT_color_buffer_float', 'EXT_color_buffer_half_float']; }
    getExtension(name) {
      const gl = this;
      switch (String(name)) {
        case 'OES_element_index_uint': case 'OES_texture_float': case 'OES_texture_float_linear': case 'EXT_color_buffer_float':
        case 'EXT_color_buffer_half_float': case 'OES_texture_half_float_linear': case 'EXT_float_blend': return {};
        case 'OES_standard_derivatives': return { FRAGMENT_SHADER_DERIVATIVE_HINT_OES: 0x8B8B };
        case 'OES_texture_half_float': return { HALF_FLOAT_OES: 0x8D61 };
        case 'EXT_blend_minmax': return { MIN_EXT: 0x8007, MAX_EXT: 0x8008 };
        case 'WEBGL_lose_context': return { loseContext() {}, restoreContext() {} };
        case 'ANGLE_instanced_arrays': return { VERTEX_ATTRIB_ARRAY_DIVISOR_ANGLE: 0x88FE,
          drawArraysInstancedANGLE: (m, f, c, n) => gl.drawArraysInstanced(m, f, c, n),
          drawElementsInstancedANGLE: (m, c, t, o, n) => gl.drawElementsInstanced(m, c, t, o, n),
          vertexAttribDivisorANGLE: (i, d) => gl.vertexAttribDivisor(i, d) };
        case 'OES_vertex_array_object': return { VERTEX_ARRAY_BINDING_OES: 0x85B5,
          createVertexArrayOES: () => gl.createVertexArray(), deleteVertexArrayOES: v => gl.deleteVertexArray(v),
          bindVertexArrayOES: v => gl.bindVertexArray(v), isVertexArrayOES: v => v instanceof WebGLVertexArrayObject };
        default: return null;
      }
    }
    getShaderPrecisionFormat(shaderType, precisionType) {
      return precisionType >= 0x8DF3 ? new WebGLShaderPrecisionFormat(31, 0) : new WebGLShaderPrecisionFormat(127, 23);
    }
    getParameter(p) {
      const s = this._s, v2 = this._v2;
      switch (p) {
        case 0x1F00: return 'SexOS';
        case 0x1F01: return 'sex-gl';
        case 0x1F02: return v2 ? 'WebGL 2.0 (sex-gl)' : 'WebGL 1.0 (sex-gl)';
        case 0x8B8C: return v2 ? 'WebGL GLSL ES 3.00 (sex-gl)' : 'WebGL GLSL ES 1.0 (sex-gl)';
        case 0xD33: case 0x851C: case 0x84E8: case 0x8073: return 4096;
        case 0x8869: return 16;
        case 0x8872: case 0x8B4C: return 16;
        case 0x8B4D: return 32;
        case 0x8DFC: return 15;
        case 0x8DFB: case 0x8DFD: return 1024;
        case 0x8B4A: case 0x8B49: return 4096;
        case 0x8D57: return 0;
        case 0x8824: case 0x8CDF: return 1;
        case 0x88FF: return 256;
        case 0x8A2F: return 24;
        case 0x8A30: return 16384;
        case 0x84FD: return 2;
        case 0x9111: return 0;
        case 0x8D6B: return 4294967295;
        case 0xD3A: return new Int32Array([4096, 4096]);
        case 0x846E: return new Float32Array([1, 1]);
        case 0x846D: return new Float32Array([1, 256]);
        case 0xBA2: return new Int32Array(s.viewport);
        case 0xC10: return new Int32Array(s.scissor);
        case 0xC22: return new Float32Array(s.clear);
        case 0xB73: return s.depthClear;
        case 0xB74: return s.depthFunc;
        case 0xB72: return s.depthMask;
        case 0xC23: return s.colorMask.slice();
        case 0x8B8D: return s.program;
        case 0x8894: return s.array;
        case 0x8895: return s.elem.get(s.vao) || null;
        case 0x85B5: return s.vao;
        case 0x84E0: return 0x84C0 + s.unit;
        case 0x8069: return (s.tex[s.unit] || {})[0xDE1] || null;
        case 0x8514: return (s.tex[s.unit] || {})[0x8513] || null;
        case 0x8CA6: return s.fb;
        case 0x8CAA: return s.readFb || null;
        case 0x8CA7: return s.rb;
        case 0xB45: return s.cullFace;
        case 0xB46: return s.frontFace;
        case 0xB21: return s.lineWidth;
        case 0xD52: case 0xD53: case 0xD54: case 0xD55: return 8;
        case 0xD56: return 24;
        case 0xD57: return 0;
        case 0xD50: return 4;
        case 0x80A8: case 0x80A9: return 0;
        case 0xCF5: return s.align;
        case 0x9240: return s.flipY;
        case 0x9241: return s.premul;
        case 0x9243: return 0x9244;
        case 0x80C9: return s.blend[0];
        case 0x80C8: return s.blend[1];
        case 0x80CB: return s.blend[2];
        case 0x80CA: return s.blend[3];
        case 0x8009: return s.blendEq[0];
        case 0x883D: return s.blendEq[1];
        case 0x86A3: return new Uint32Array(0);
        case 0x8B9A: return 0x1401;
        case 0x8B9B: return 0x1908;
        default: return (p in s.enabled) ? s.enabled[p] : null;
      }
    }
    getError() { return +this._ask('error'); }
    // State
    enable(cap) { this._s.enabled[cap] = true; this._c('en', cap, 1); }
    disable(cap) { this._s.enabled[cap] = false; this._c('en', cap, 0); }
    isEnabled(cap) { return !!this._s.enabled[cap]; }
    viewport(x, y, w, h) { this._s.viewport = [x | 0, y | 0, w | 0, h | 0]; this._c('vp', x | 0, y | 0, w | 0, h | 0); }
    scissor(x, y, w, h) { this._s.scissor = [x | 0, y | 0, w | 0, h | 0]; this._c('sc', x | 0, y | 0, w | 0, h | 0); }
    clearColor(r, g, b, a) { this._s.clear = [+r, +g, +b, +a]; this._c('cc', numStr(r), numStr(g), numStr(b), numStr(a)); }
    clearDepth(d) { this._s.depthClear = +d; this._c('cd', numStr(d)); }
    clearStencil() {}
    colorMask(r, g, b, a) { this._s.colorMask = [!!r, !!g, !!b, !!a]; this._c('cm', +!!r, +!!g, +!!b, +!!a); }
    depthMask(m) { this._s.depthMask = !!m; this._c('dm', +!!m); }
    depthFunc(f) { this._s.depthFunc = f; this._c('df', f); }
    depthRange(n, f) { this._c('dr', numStr(n), numStr(f)); }
    cullFace(m) { this._s.cullFace = m; this._c('cf', m); }
    frontFace(m) { this._s.frontFace = m; this._c('ff', m); }
    blendFunc(s, d) { this.blendFuncSeparate(s, d, s, d); }
    blendFuncSeparate(sr, dr, sa, da) { this._s.blend = [sr, dr, sa, da]; this._c('bf', sr, dr, sa, da); }
    blendEquation(m) { this.blendEquationSeparate(m, m); }
    blendEquationSeparate(r, a) { this._s.blendEq = [r, a]; this._c('be', r, a); }
    blendColor(r, g, b, a) { this._c('bc', numStr(r), numStr(g), numStr(b), numStr(a)); }
    polygonOffset(f, u) { this._c('po', numStr(f), numStr(u)); }
    pixelStorei(p, v) {
      if (p === 0x9240) this._s.flipY = !!v;
      if (p === 0x9241) this._s.premul = !!v;
      if (p === 0xCF5) this._s.align = v;
      if (p === 0xCF2) this._s.rowLength = v | 0;
      if (p === 0xCF3) this._s.skipRows = v | 0;
      if (p === 0xCF4) this._s.skipPixels = v | 0;
      this._c('ps', p, typeof v === 'boolean' ? +v : v | 0);
    }
    lineWidth(w) { this._s.lineWidth = +w; }
    hint() {} sampleCoverage() {} stencilFunc() {} stencilFuncSeparate() {} stencilMask() {} stencilMaskSeparate() {}
    stencilOp() {} stencilOpSeparate() {}
    clear(mask) { this._c('cl', mask); }
    flush() { this._flush(); }
    finish() { this._flush(); }
    // Buffers
    createBuffer() { return new WebGLBuffer(); }
    deleteBuffer(b) { if (b) this._c('db', b._id); }
    isBuffer(b) { return b instanceof WebGLBuffer; }
    bindBuffer(t, b) {
      if (t === 0x8892) this._s.array = b; else if (t === 0x8893) this._s.elem.set(this._s.vao, b);
      this._c('bb', t, b ? b._id : 0);
    }
    bufferData(t, data, usage, srcOffset, length) {
      if (typeof data === 'number') { this._c('bz', t, data | 0); return; }
      let v = data;
      if (srcOffset !== undefined && ArrayBuffer.isView(v)) v = v.subarray(srcOffset, length ? srcOffset + length : undefined);
      const [o, l] = this._data(v);
      this._c('bd', t, o, l);
    }
    bufferSubData(t, offset, data, srcOffset, length) {
      let v = data;
      if (srcOffset !== undefined && ArrayBuffer.isView(v)) v = v.subarray(srcOffset, length ? srcOffset + length : undefined);
      const [o, l] = this._data(v);
      this._c('bs', t, offset | 0, o, l);
    }
    getBufferParameter() { return 0; }
    // Vertex attributes
    enableVertexAttribArray(i) { this._c('ea', i, 1); }
    disableVertexAttribArray(i) { this._c('ea', i, 0); }
    vertexAttribPointer(i, size, type, norm, stride, offset) { this._c('ap', i, size, type, +!!norm, stride | 0, offset | 0); }
    vertexAttrib1f(i, x) { this._c('va', i, numStr(x), 0, 0, 1); }
    vertexAttrib2f(i, x, y) { this._c('va', i, numStr(x), numStr(y), 0, 1); }
    vertexAttrib3f(i, x, y, z) { this._c('va', i, numStr(x), numStr(y), numStr(z), 1); }
    vertexAttrib4f(i, x, y, z, w) { this._c('va', i, numStr(x), numStr(y), numStr(z), numStr(w)); }
    vertexAttrib1fv(i, v) { this.vertexAttrib1f(i, v[0]); }
    vertexAttrib2fv(i, v) { this.vertexAttrib2f(i, v[0], v[1]); }
    vertexAttrib3fv(i, v) { this.vertexAttrib3f(i, v[0], v[1], v[2]); }
    vertexAttrib4fv(i, v) { this.vertexAttrib4f(i, v[0], v[1], v[2], v[3]); }
    getVertexAttrib() { return null; }
    getVertexAttribOffset() { return 0; }
    // Vertex arrays (WebGL 2, and OES_vertex_array_object)
    createVertexArray() { return new WebGLVertexArrayObject(); }
    deleteVertexArray(v) { if (v) this._c('dva', v._id); }
    bindVertexArray(v) { this._s.vao = v || null; this._c('ba', v ? v._id : 0); }
    isVertexArray(v) { return v instanceof WebGLVertexArrayObject; }
    vertexAttribDivisor(i, d) { this._c('ad', i, d); }
    // Textures
    createTexture() { return new WebGLTexture(); }
    deleteTexture(t) { if (t) this._c('dt', t._id); }
    isTexture(t) { return t instanceof WebGLTexture; }
    activeTexture(u) { this._s.unit = u - 0x84C0; this._c('at', u); }
    bindTexture(t, tex) {
      (this._s.tex[this._s.unit] || (this._s.tex[this._s.unit] = {}))[t] = tex;
      this._c('bt', t, tex ? tex._id : 0);
    }
    _boundTex(target) {
      const t = (target >= 0x8515 && target <= 0x851A) ? 0x8513 : target;
      const tex = (this._s.tex[this._s.unit] || {})[t];
      return tex ? tex._id : 0;
    }
    texParameteri(t, p, v) { this._c('tp', t, p, v); }
    texParameterf(t, p, v) { this._c('tp', t, p, v | 0); }
    getTexParameter() { return null; }
    generateMipmap(t) { this._c('gm', t); }
    _picture(target, src) {
      if (src && src.tagName === 'IMG') { this._c('tP', target, this._boundTex(target), src.getAttribute('src') || ''); return true; }
      if (src && src.tagName === 'CANVAS') { this._c('tP', target, this._boundTex(target), 'canvas:' + src._id); return true; }
      if (src && typeof src.width === 'number' && src.data && ArrayBuffer.isView(src.data)) {
        const [o, l] = this._data(src.data);
        this._c('ti', target, 0, src.width, src.height, 0x1908, 0x1401, o, l);
        return true;
      }
      return false;
    }
    texImage2D(target, level, ifmt, a, b, c, d, e, f) {
      if (arguments.length === 6) { if (!this._picture(target, c)) this._c('ti', target, level, 0, 0, 0x1908, 0x1401); return; }
      const [w, h, fmt, type, pixels] = [a, b, d, e, f];
      if (pixels && !ArrayBuffer.isView(pixels) && !(pixels instanceof ArrayBuffer)) { this._picture(target, pixels); return; }
      const f2 = fmt === 0x1908 || fmt === 0x1907 ? fmt : (ifmt === 0x1907 || ifmt === 0x8051 ? 0x1907 : fmt);
      if (pixels) { const [o, l] = this._data(pixels); this._c('ti', target, level, w, h, f2, type, o, l); }
      else this._c('ti', target, level, w, h, f2, type);
    }
    texSubImage2D(target, level, x, y, a, b, c, d, e) {
      if (arguments.length === 7) return; // pictures into part of a texture: not yet
      const [w, h, fmt, type, pixels] = [a, b, c, d, e];
      if (!pixels || !(ArrayBuffer.isView(pixels) || pixels instanceof ArrayBuffer)) return;
      const st = this._s;
      if (st.rowLength || st.skipRows || st.skipPixels) {
        // A window into a larger image: copy its rows out tightly packed.
        const comps = { 0x1908: 4, 0x8D99: 4, 0x1907: 3, 0x8D98: 3, 0x8227: 2, 0x8228: 2, 0x190A: 2 }[fmt] || 1;
        const size = { 0x1406: 4, 0x1404: 4, 0x1405: 4, 0x140B: 2, 0x8D61: 2, 0x1402: 2, 0x1403: 2 }[type] || 1;
        const bpp = [0x8363, 0x8033, 0x8034].includes(type) ? 2 : comps * size;
        const src = new Uint8Array(pixels.buffer || pixels, pixels.byteOffset || 0, pixels.byteLength);
        const al = st.align || 4, rowBytes = (st.rowLength || w) * bpp, stride = Math.ceil(rowBytes / al) * al;
        const out = new Uint8Array(w * h * bpp);
        for (let r = 0; r < h; r++) {
          const from = (st.skipRows + r) * stride + st.skipPixels * bpp;
          out.set(src.subarray(from, from + w * bpp), r * w * bpp);
        }
        const saved = st.align;
        if (saved !== 1) this._c('ps', 0xCF5, 1);
        const [o, l] = this._data(out);
        this._c('ts', target, level, x, y, w, h, fmt, type, o, l);
        if (saved !== 1) this._c('ps', 0xCF5, saved);
        return;
      }
      const [o, l] = this._data(pixels);
      this._c('ts', target, level, x, y, w, h, fmt, type, o, l);
    }
    texStorage2D(target, levels, ifmt, w, h) {
      const sized = this._sized(ifmt);
      if (target === 0x8513) { for (let f = 0; f < 6; f++) this._c('ti', 0x8515 + f, 0, w, h, sized[0], sized[1]); }
      else this._c('ti', target, 0, w, h, sized[0], sized[1]);
    }
    texImage3D(target, level, ifmt, w, h, depth, border, fmt, type, pixels) {
      if (pixels && !(ArrayBuffer.isView(pixels) || pixels instanceof ArrayBuffer)) return; // pictures: not yet
      const sized = fmt === 0x1908 && type === 0x1401 ? this._sized(ifmt) : [fmt, type];
      if (pixels) { const [o, l] = this._data(pixels); this._c('ti3', target, level, w, h, depth, sized[0], sized[1], o, l); }
      else this._c('ti3', target, level, w, h, depth, sized[0], sized[1]);
    }
    texStorage3D(target, levels, ifmt, w, h, depth) {
      const sized = this._sized(ifmt);
      this._c('ti3', target, 0, w, h, depth, sized[0], sized[1]);
    }
    texSubImage3D(target, level, x, y, z, w, h, d, fmt, type, pixels) {
      if (!pixels || !(ArrayBuffer.isView(pixels) || pixels instanceof ArrayBuffer)) return;
      const [o, l] = this._data(pixels);
      this._c('ts3', target, level, x, y, z, w, h, d, fmt, type, o, l);
    }
    // The client format and type that keep a sized internal format's texels: float and
    // integer formats keep theirs, the rest are RGBA8.
    _sized(ifmt) {
      return {
        0x822E: [0x1903, 0x1406], 0x8230: [0x8227, 0x1406], 0x8815: [0x1907, 0x1406], 0x8814: [0x1908, 0x1406],
        0x822D: [0x1903, 0x140B], 0x822F: [0x8227, 0x140B], 0x881B: [0x1907, 0x140B], 0x881A: [0x1908, 0x140B],
        0x8232: [0x8D94, 0x1401], 0x8234: [0x8D94, 0x1403], 0x8236: [0x8D94, 0x1405], 0x8238: [0x8228, 0x1401],
        0x823A: [0x8228, 0x1403], 0x823C: [0x8228, 0x1405], 0x8D7C: [0x8D99, 0x1401], 0x8D76: [0x8D99, 0x1403],
        0x8D70: [0x8D99, 0x1405], 0x8D71: [0x8D98, 0x1405], 0x8D7D: [0x8D98, 0x1401], 0x8D77: [0x8D98, 0x1403],
        0x8231: [0x8D94, 0x1400], 0x8233: [0x8D94, 0x1402], 0x8235: [0x8D94, 0x1404], 0x8237: [0x8228, 0x1400],
        0x8239: [0x8228, 0x1402], 0x823B: [0x8228, 0x1404], 0x8D8E: [0x8D99, 0x1400], 0x8D88: [0x8D99, 0x1402],
        0x8D82: [0x8D99, 0x1404], 0x8D83: [0x8D98, 0x1404], 0x8D8F: [0x8D98, 0x1400], 0x8D89: [0x8D98, 0x1402],
      }[ifmt] || [0x1908, 0x1401];
    }
    copyTexImage2D() {} copyTexSubImage2D() {} compressedTexImage2D() {} compressedTexSubImage2D() {}
    // Shaders and programs
    createShader(type) { const s = new WebGLShader(); s._type = type; s._src = ''; this._c('cs', s._id, type); return s; }
    shaderSource(s, src) { s._src = String(src); this._ask('source', s._id, s._src); }
    getShaderSource(s) { return s ? s._src : null; }
    compileShader(s) { this._c('co', s._id); s._status = null; }
    getShaderParameter(s, p) {
      if (p === 0x8B4F) return s._type;
      if (p === 0x8B80) return false;
      if (s._status == null) { const r = this._ask('shader', s._id).split('\x1f'); s._status = r[0] === '1'; s._log = r[1] || ''; }
      return p === 0x8B81 ? s._status : null;
    }
    getShaderInfoLog(s) { this.getShaderParameter(s, 0x8B81); return s._log || ''; }
    deleteShader(s) { if (s) this._c('ds', s._id); }
    isShader(s) { return s instanceof WebGLShader; }
    createProgram() { const p = new WebGLProgram(); p._shaders = []; this._c('cp', p._id); return p; }
    attachShader(p, s) { p._shaders.push(s); this._c('as', p._id, s._id); }
    detachShader(p, s) { p._shaders = p._shaders.filter(x => x !== s); }
    getAttachedShaders(p) { return p._shaders.slice(); }
    bindAttribLocation(p, i, name) { this._c('bl', p._id, i, String(name).replace(/[\x1e\x1f]/g, '')); }
    linkProgram(p) { this._c('lp', p._id); p._info = null; p._locs = new Map(); }
    _info(p) {
      if (!p._info) {
        const r = this._ask('program', p._id).split('\x1f');
        p._info = { ok: r[0] === '1', log: r[1] || '', uniforms: +r[2] || 0, attribs: +r[3] || 0, u: null, a: null };
      }
      return p._info;
    }
    getProgramParameter(p, q) {
      const i = this._info(p);
      switch (q) {
        case 0x8B82: return i.ok;
        case 0x8B83: return i.ok;
        case 0x8B80: return false;
        case 0x8B85: return p._shaders.length;
        case 0x8B86: return i.uniforms;
        case 0x8B89: return i.attribs;
        case 0x8A36: return 0;
        default: return null;
      }
    }
    getProgramInfoLog(p) { return this._info(p).log; }
    validateProgram() {}
    useProgram(p) { this._s.program = p || null; this._c('up', p ? p._id : 0); }
    deleteProgram(p) { if (p) this._c('dp', p._id); }
    isProgram(p) { return p instanceof WebGLProgram; }
    getActiveUniform(p, i) {
      const info = this._info(p);
      if (!info.u) info.u = (this._ask('uniforms', p._id) || '').split('\x1e').filter(Boolean).map(r => r.split('\x1f'));
      const r = info.u[i];
      return r ? new WebGLActiveInfo(r[0], +r[1], +r[2]) : null;
    }
    getActiveAttrib(p, i) {
      const info = this._info(p);
      if (!info.a) info.a = (this._ask('attribs', p._id) || '').split('\x1e').filter(Boolean).map(r => r.split('\x1f'));
      const r = info.a[i];
      return r ? new WebGLActiveInfo(r[0], +r[1], +r[2]) : null;
    }
    getAttribLocation(p, name) { return +this._ask('aloc', p._id, name); }
    getUniformLocation(p, name) {
      if (!p._locs) p._locs = new Map();
      if (p._locs.has(name)) return p._locs.get(name);
      const r = this._ask('uloc', p._id, name);
      const loc = r === '' || r == null ? null : new WebGLUniformLocation(p, +r);
      p._locs.set(name, loc);
      return loc;
    }
    getUniform() { return null; }
    getUniformBlockIndex() { return 0xFFFFFFFF; }
    uniformBlockBinding() {} bindBufferBase() {} bindBufferRange() {}
    getActiveUniformBlockParameter() { return null; } getActiveUniformBlockName() { return null; }
    getUniformIndices() { return null; } getActiveUniforms() { return null; }
    _uf(l, vals) { if (!l) return; const [o, n] = this._data(Float32Array.from(vals)); this._c('uf', l._loc, o, n); }
    _ui(l, vals) { if (!l) return; const [o, n] = this._data(Int32Array.from(vals)); this._c('ui', l._loc, o, n); }
    uniform1f(l, x) { this._uf(l, [x]); }
    uniform2f(l, x, y) { this._uf(l, [x, y]); }
    uniform3f(l, x, y, z) { this._uf(l, [x, y, z]); }
    uniform4f(l, x, y, z, w) { this._uf(l, [x, y, z, w]); }
    uniform1i(l, x) { this._ui(l, [x]); }
    uniform2i(l, x, y) { this._ui(l, [x, y]); }
    uniform3i(l, x, y, z) { this._ui(l, [x, y, z]); }
    uniform4i(l, x, y, z, w) { this._ui(l, [x, y, z, w]); }
    uniform1ui(l, x) { this._ui(l, [x]); }
    uniform2ui(l, x, y) { this._ui(l, [x, y]); }
    uniform3ui(l, x, y, z) { this._ui(l, [x, y, z]); }
    uniform4ui(l, x, y, z, w) { this._ui(l, [x, y, z, w]); }
    uniform1fv(l, v) { this._uf(l, v); } uniform2fv(l, v) { this._uf(l, v); } uniform3fv(l, v) { this._uf(l, v); } uniform4fv(l, v) { this._uf(l, v); }
    uniform1iv(l, v) { this._ui(l, v); } uniform2iv(l, v) { this._ui(l, v); } uniform3iv(l, v) { this._ui(l, v); } uniform4iv(l, v) { this._ui(l, v); }
    uniform1uiv(l, v) { this._ui(l, v); } uniform2uiv(l, v) { this._ui(l, v); } uniform3uiv(l, v) { this._ui(l, v); } uniform4uiv(l, v) { this._ui(l, v); }
    _mat(l, t, v, n) {
      if (!l) return;
      if (!t) return this._uf(l, v);
      const out = [];
      for (let m = 0; m + n * n <= v.length; m += n * n) for (let c = 0; c < n; c++) for (let r = 0; r < n; r++) out.push(v[m + r * n + c]);
      this._uf(l, out);
    }
    uniformMatrix2fv(l, t, v) { this._mat(l, t, v, 2); }
    uniformMatrix3fv(l, t, v) { this._mat(l, t, v, 3); }
    uniformMatrix4fv(l, t, v) { this._mat(l, t, v, 4); }
    // Drawing
    drawArrays(mode, first, count) { this._c('da', mode, first | 0, count | 0); }
    drawElements(mode, count, type, offset) { this._c('de', mode, count | 0, type, offset | 0); }
    drawArraysInstanced(mode, first, count, n) { this._c('dai', mode, first | 0, count | 0, n | 0); }
    drawElementsInstanced(mode, count, type, offset, n) { this._c('dei', mode, count | 0, type, offset | 0, n | 0); }
    drawRangeElements(mode, start, end, count, type, offset) { this.drawElements(mode, count, type, offset); }
    drawBuffers() {} readBuffer() {} invalidateFramebuffer() {} clearBufferfv() {} clearBufferiv() {}
    blitFramebuffer(sx0, sy0, sx1, sy1, dx0, dy0, dx1, dy1, mask) {
      if (!(mask & 0x4000)) return;
      const r = this._s.readFb, d = this._s.fb;
      this._c('blt', r ? r._id : 0, d ? d._id : 0, sx0 | 0, sy0 | 0, sx1 | 0, sy1 | 0, dx0 | 0, dy0 | 0, dx1 | 0, dy1 | 0);
    }
    clearBufferuiv() {} clearBufferfi() {}
    readPixels(x, y, w, h, format, type, out) {
      if (!out) return;
      const r = this._s.readFb, d = this._s.fb, other = (r || null) !== (d || null);
      if (other) this._c('fb', r ? r._id : 0);
      this._flush();
      const buf = H(OP.GL_QUERY | 0x10000, this.canvas._id, 'read', String(x | 0), String(y | 0), String(w | 0), String(h | 0));
      if (other) this._c('fb', d ? d._id : 0);
      if (buf instanceof ArrayBuffer) {
        const src = new Uint8Array(buf);
        const dst = bytesOf(out);
        dst.set(src.subarray(0, Math.min(src.length, dst.length)));
      }
    }
    // Framebuffers: draws land in the attached texture (render to texture) or colour
    // renderbuffer; each framebuffer has its own depth buffer.
    createFramebuffer() { return new WebGLFramebuffer(); }
    bindFramebuffer(t, f) {
      f = f || null;
      if (t !== 0x8CA9) this._s.readFb = f;   // READ_FRAMEBUFFER or FRAMEBUFFER
      if (t === 0x8CA8) return;
      if (this._s.fb !== f) { this._s.fb = f; this._c('fb', f ? f._id : 0); }
    }
    deleteFramebuffer(f) {
      if (!f) return;
      if (this._s.fb === f) this._s.fb = null;
      if (this._s.readFb === f) this._s.readFb = null;
      this._c('dfb', f._id);
    }
    isFramebuffer(f) { return f instanceof WebGLFramebuffer; }
    framebufferTexture2D(t, attachment, textarget, tex) { if (t !== 0x8CA8) this._c('ft', attachment, textarget, tex ? tex._id : 0); }
    framebufferTextureLayer() {}
    framebufferRenderbuffer(t, attachment, rt, rb) { if (t !== 0x8CA8) this._c('fr', attachment, rb ? rb._id : 0); }
    checkFramebufferStatus() { return 0x8CD5; }
    getFramebufferAttachmentParameter() { return null; }
    createRenderbuffer() { return new WebGLRenderbuffer(); }
    bindRenderbuffer(t, r) { this._s.rb = r || null; this._c('rb', r ? r._id : 0); }
    deleteRenderbuffer(r) { if (r) this._c('drb', r._id); }
    isRenderbuffer(r) { return r instanceof WebGLRenderbuffer; }
    renderbufferStorage(t, fmt, w, h) { if (this._s.rb) { this._s.rb._w = w; this._s.rb._h = h; } this._c('rs', w | 0, h | 0); }
    renderbufferStorageMultisample(t, samples, fmt, w, h) { this.renderbufferStorage(t, fmt, w, h); }
    getRenderbufferParameter(t, p) { const r = this._s.rb; return !r ? 0 : p === 0x8D42 ? r._w | 0 : p === 0x8D43 ? r._h | 0 : 0; }
    // WebGL 2 odds and ends
    createQuery() { return new WebGLQuery(); } deleteQuery() {} beginQuery() {} endQuery() {} getQueryParameter() { return 0; }
    createSampler() { return new WebGLSampler(); } deleteSampler() {} bindSampler() {} samplerParameteri() {} samplerParameterf() {}
    fenceSync() { return new WebGLSync(); } deleteSync() {} clientWaitSync() { return 0x911A; } waitSync() {} getSyncParameter() { return 0x9119; }
    createTransformFeedback() { return new WebGLTransformFeedback(); } bindTransformFeedback() {} beginTransformFeedback() {}
    endTransformFeedback() {} transformFeedbackVaryings() {} deleteTransformFeedback() {}
    vertexAttribIPointer(i, size, type, stride, offset) { this._c('aip', i, size, type, stride | 0, offset | 0); }
    vertexAttribI4i(i, x, y, z, w) { this.vertexAttrib4f(i, x, y, z, w); } vertexAttribI4ui(i, x, y, z, w) { this.vertexAttrib4f(i, x, y, z, w); }
    getBufferSubData() {} copyBufferSubData() {} getIndexedParameter() { return null; } getInternalformatParameter() { return new Int32Array(0); }
  }
  for (let i = 0; i < 32; i++) GL_CONSTANTS['TEXTURE' + i] = 0x84C0 + i;
  for (const k in GL_CONSTANTS) { WebGLRenderingContext.prototype[k] = GL_CONSTANTS[k]; WebGLRenderingContext[k] = GL_CONSTANTS[k]; }
  class WebGL2RenderingContext extends WebGLRenderingContext {
    constructor(canvas, attrs) { super(canvas, attrs, true); }
  }
  for (const k in GL2_CONSTANTS) { WebGL2RenderingContext.prototype[k] = GL2_CONSTANTS[k]; WebGL2RenderingContext[k] = GL2_CONSTANTS[k]; }
  for (const k in GL_CONSTANTS) WebGL2RenderingContext[k] = GL_CONSTANTS[k];

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

  // ---- WebGPU ----
  // Render pipelines on the same software renderer as WebGL. Buffers and textures live
  // here and are copied into a canvas's GL context when a pass there uses them; command
  // encoders record passes, and queue.submit replays them as GL commands, with WGSL
  // programs ('gp') and bind group entries ('gb' buffers, 'gx' textures, 'gs' samplers).
  const GPU_BLEND = { zero: 0, one: 1, src: 0x300, 'one-minus-src': 0x301, 'src-alpha': 0x302, 'one-minus-src-alpha': 0x303,
    dst: 0x306, 'one-minus-dst': 0x307, 'dst-alpha': 0x304, 'one-minus-dst-alpha': 0x305, 'src-alpha-saturated': 0x308,
    constant: 0x8001, 'one-minus-constant': 0x8002 };
  const GPU_BLEND_OP = { add: 0x8006, subtract: 0x800A, 'reverse-subtract': 0x800B, min: 0x8007, max: 0x8008 };
  const GPU_COMPARE = { never: 0x200, less: 0x201, equal: 0x202, 'less-equal': 0x203, greater: 0x204, 'not-equal': 0x205,
    'greater-equal': 0x206, always: 0x207 };
  const GPU_TOPOLOGY = { 'point-list': 0, 'line-list': 1, 'line-strip': 3, 'triangle-list': 4, 'triangle-strip': 5 };
  // Vertex formats: components, GL type, normalized.
  const GPU_VERTEX = { float32: [1, 0x1406, 0], float32x2: [2, 0x1406, 0], float32x3: [3, 0x1406, 0], float32x4: [4, 0x1406, 0],
    float16x2: [2, 0x140B, 0], float16x4: [4, 0x140B, 0], unorm8x2: [2, 0x1401, 1], unorm8x4: [4, 0x1401, 1],
    snorm8x2: [2, 0x1400, 1], snorm8x4: [4, 0x1400, 1], uint8x2: [2, 0x1401, 0], uint8x4: [4, 0x1401, 0],
    sint8x2: [2, 0x1400, 0], sint8x4: [4, 0x1400, 0], unorm16x2: [2, 0x1403, 1], unorm16x4: [4, 0x1403, 1],
    snorm16x2: [2, 0x1402, 1], snorm16x4: [4, 0x1402, 1], uint16x2: [2, 0x1403, 0], uint16x4: [4, 0x1403, 0],
    sint16x2: [2, 0x1402, 0], sint16x4: [4, 0x1402, 0], uint32: [1, 0x1405, 0], uint32x2: [2, 0x1405, 0],
    uint32x3: [3, 0x1405, 0], uint32x4: [4, 0x1405, 0], sint32: [1, 0x1404, 0], sint32x2: [2, 0x1404, 0],
    sint32x3: [3, 0x1404, 0], sint32x4: [4, 0x1404, 0] };
  const GPU_FILTER = { nearest: 0x2600, linear: 0x2601 };
  const GPU_WRAP = { repeat: 0x2901, 'clamp-to-edge': 0x812F, 'mirror-repeat': 0x8370 };
  const GPUBufferUsage = { MAP_READ: 1, MAP_WRITE: 2, COPY_SRC: 4, COPY_DST: 8, INDEX: 16, VERTEX: 32, UNIFORM: 64, STORAGE: 128,
    INDIRECT: 256, QUERY_RESOLVE: 512 };
  const GPUTextureUsage = { COPY_SRC: 1, COPY_DST: 2, TEXTURE_BINDING: 4, STORAGE_BINDING: 8, RENDER_ATTACHMENT: 16 };
  const GPUShaderStage = { VERTEX: 1, FRAGMENT: 2, COMPUTE: 4 };
  const GPUMapMode = { READ: 1, WRITE: 2 };
  const GPUColorWrite = { RED: 1, GREEN: 2, BLUE: 4, ALPHA: 8, ALL: 15 };
  const gpuSize = s => Array.isArray(s) || (s && typeof s[Symbol.iterator] === 'function' && !('width' in s))
    ? [...s].concat([1, 1, 1]).slice(0, 3).map(Number) : [+(s.width || 0), +(s.height || 1), +(s.depthOrArrayLayers || 1)];
  const gpuOrigin = o => !o ? [0, 0, 0] : Array.isArray(o) || ArrayBuffer.isView(o)
    ? [...o].concat([0, 0, 0]).slice(0, 3).map(Number) : [+(o.x || 0), +(o.y || 0), +(o.z || 0)];
  const gpuColor = c => Array.isArray(c) ? [c[0] || 0, c[1] || 0, c[2] || 0, c[3] === undefined ? 1 : c[3]]
    : [+(c && c.r) || 0, +(c && c.g) || 0, +(c && c.b) || 0, c && c.a !== undefined ? +c.a : 1];

  class GPUObjectBase { constructor(d) { this.label = (d && d.label) || ''; this._id = glNextId++; } }
  class GPUBuffer extends GPUObjectBase {
    constructor(d) {
      super(d);
      this.size = Math.max(0, +d.size || 0); this.usage = d.usage | 0;
      this._bytes = new Uint8Array((this.size + 3) & ~3); this._ver = 1; this._in = new Map();
      this.mapState = d.mappedAtCreation ? 'mapped' : 'unmapped';
    }
    getMappedRange(offset = 0, size) {
      const end = size === undefined ? this.size : offset + size;
      // A copy of the range; unmap writes it back.
      const ab = new ArrayBuffer(end - offset);
      new Uint8Array(ab).set(this._bytes.subarray(offset, end));
      (this._maps || (this._maps = [])).push([ab, offset]);
      return ab;
    }
    unmap() {
      for (const [ab, off] of this._maps || []) this._bytes.set(new Uint8Array(ab), off);
      this._maps = null; this._ver++; this.mapState = 'unmapped';
    }
    mapAsync() { this.mapState = 'mapped'; return Promise.resolve(); }
    destroy() {}
    _upload(gl) {
      if (this._in.get(gl) === this._ver) return;
      this._in.set(gl, this._ver);
      const [o, l] = gl._data(this._bytes);
      gl._c('bb', 0x8892, this._id); gl._c('bd', 0x8892, o, l);
    }
  }
  class GPUTexture extends GPUObjectBase {
    constructor(d, canvasCtx) {
      super(d);
      [this.width, this.height, this.depthOrArrayLayers] = gpuSize(d.size || [1, 1, 1]);
      this.format = d.format || 'rgba8unorm'; this.usage = d.usage | 0; this.dimension = d.dimension || '2d';
      this.mipLevelCount = d.mipLevelCount || 1; this.sampleCount = d.sampleCount || 1;
      this._canvas = canvasCtx || null; this._px = null; this._pic = null; this._ver = 1; this._in = new Map();
    }
    createView(d) { return new GPUTextureView(this, d); }
    destroy() {}
    _upload(gl, unit) {
      gl._c('at', 0x84C0 + unit); gl._c('bt', 0xDE1, this._id);
      if (this._in.get(gl) === this._ver) return;
      this._in.set(gl, this._ver);
      if (this._pic) { gl._c('ps', 0x9240, this._pic[1] ? 1 : 0); gl._c('ps', 0x9241, 0); gl._c('tP', 0xDE1, this._id, this._pic[0]); gl._c('ps', 0x9240, 0); return; }
      const px = this._px || new Uint8Array(this.width * this.height * 4);
      const [o, l] = gl._data(px);
      gl._c('ps', 0xCF5, 1);
      gl._c('ti', 0xDE1, 0, this.width, this.height, 0x1908, 0x1401, o, l);
    }
  }
  class GPUTextureView extends GPUObjectBase { constructor(t, d) { super(d); this.texture = t; } }
  class GPUSampler extends GPUObjectBase {
    constructor(d) {
      super(d); d = d || {};
      this._f = [GPU_FILTER[d.minFilter] || 0x2600, GPU_FILTER[d.magFilter] || 0x2600,
        GPU_WRAP[d.addressModeU] || 0x812F, GPU_WRAP[d.addressModeV] || 0x812F];
    }
  }
  class GPUShaderModule extends GPUObjectBase {
    constructor(d) { super(d); this._code = String(d.code || ''); }
    getCompilationInfo() { return Promise.resolve({ messages: [] }); }
  }
  class GPUBindGroupLayout extends GPUObjectBase {}
  class GPUPipelineLayout extends GPUObjectBase {}
  class GPUBindGroup extends GPUObjectBase { constructor(d) { super(d); this._entries = d.entries || []; } }
  class GPUQuerySet extends GPUObjectBase { destroy() {} }
  class GPURenderPipeline extends GPUObjectBase {
    constructor(d) {
      super(d); this._d = d; this._in = new Set();
      this._vs = d.vertex.module._code;
      this._fs = d.fragment ? d.fragment.module._code : this._vs;
    }
    getBindGroupLayout() { return new GPUBindGroupLayout(); }
    _program(gl) {
      if (this._in.has(gl)) return;
      this._in.add(gl);
      const enc = new TextEncoderLite();
      const [vo, vl] = gl._data(enc.encode(this._vs));
      const [fo, fl] = this._fs === this._vs ? [vo, vl] : gl._data(enc.encode(this._fs));
      gl._c('gp', this._id, vo, vl, this._d.vertex.entryPoint || '', fo, fl, (this._d.fragment && this._d.fragment.entryPoint) || '');
      const st = gl._ask('program', this._id).split('\x1f');
      if (st[0] !== '1') console.error('WebGPU pipeline:', st[1]);
    }
  }
  // UTF-8 for shader text (QuickJS has no TextEncoder).
  class TextEncoderLite {
    encode(s) {
      const out = [];
      for (const ch of String(s)) {
        let c = ch.codePointAt(0);
        if (c < 0x80) out.push(c);
        else if (c < 0x800) out.push(0xC0 | c >> 6, 0x80 | c & 63);
        else if (c < 0x10000) out.push(0xE0 | c >> 12, 0x80 | c >> 6 & 63, 0x80 | c & 63);
        else out.push(0xF0 | c >> 18, 0x80 | c >> 12 & 63, 0x80 | c >> 6 & 63, 0x80 | c & 63);
      }
      return new Uint8Array(out);
    }
  }

  class GPURenderPassEncoder {
    constructor(enc, d) { this._cmds = []; enc._passes.push({ d, cmds: this._cmds }); }
    setPipeline(p) { this._cmds.push(['pipe', p]); }
    setVertexBuffer(slot, buf, offset = 0, size) { this._cmds.push(['vb', slot, buf, offset]); }
    setIndexBuffer(buf, format, offset = 0) { this._cmds.push(['ib', buf, format, offset]); }
    setBindGroup(i, g) { this._cmds.push(['bg', i, g]); }
    setViewport(x, y, w, h, mn, mx) { this._cmds.push(['vp', x, y, w, h, mn, mx]); }
    setScissorRect(x, y, w, h) { this._cmds.push(['sc', x, y, w, h]); }
    setBlendConstant(c) { this._cmds.push(['bc', gpuColor(c)]); }
    setStencilReference() {}
    draw(n, inst = 1, first = 0) { this._cmds.push(['draw', n, inst, first]); }
    drawIndexed(n, inst = 1, first = 0, base = 0) { this._cmds.push(['drawi', n, inst, first, base]); }
    pushDebugGroup() {} popDebugGroup() {} insertDebugMarker() {}
    end() {}
  }
  class GPUCommandBuffer extends GPUObjectBase {}
  class GPUCommandEncoder extends GPUObjectBase {
    constructor(d) { super(d); this._passes = []; }
    beginRenderPass(d) { return new GPURenderPassEncoder(this, d); }
    beginComputePass() { return { setPipeline() {}, setBindGroup() {}, dispatchWorkgroups() {}, end() {} }; }
    copyBufferToBuffer(src, so, dst, doff, size) { this._passes.push({ copy: () => { dst._bytes.set(src._bytes.subarray(so, so + size), doff); dst._ver++; } }); }
    copyTextureToTexture() {} copyBufferToTexture() {} copyTextureToBuffer() {} clearBuffer() {} resolveQuerySet() {}
    pushDebugGroup() {} popDebugGroup() {} insertDebugMarker() {}
    finish() { const b = new GPUCommandBuffer(); b._passes = this._passes; return b; }
  }

  function gpuRunPass(pass) {
    const d = pass.d;
    const ca = d.colorAttachments && d.colorAttachments[0];
    const tex = ca && ca.view && (ca.view.texture || ca.view);
    const cc = tex && tex._canvas;
    if (!cc || !cc._gl) return;
    const gl = cc._gl, W = cc.canvas.width, H = cc.canvas.height;
    gl._c('vp', 0, 0, W, H); gl._c('dr', 0, 1); gl._c('en', 0xC11, 0); gl._c('cm', 1, 1, 1, 1); gl._c('dm', 1);
    let mask = 0;
    if (ca.loadOp === 'clear') { gl._c('cc', ...gpuColor(ca.clearValue || ca.clearColor).map(numStr)); mask |= 0x4000; }
    const ds = d.depthStencilAttachment;
    if (ds && ds.depthLoadOp === 'clear') { gl._c('cd', numStr(ds.depthClearValue === undefined ? 1 : ds.depthClearValue)); mask |= 0x100; }
    if (mask) gl._c('cl', mask);
    let pipe = null, index = null;
    const vbs = [], groups = [];
    const apply = () => {
      const p = pipe._d;
      pipe._program(gl);
      gl._c('up', pipe._id);
      // Vertex buffers.
      (p.vertex.buffers || []).forEach((layout, slot) => {
        const vb = vbs[slot];
        if (!layout || !vb) return;
        vb[0]._upload(gl);
        gl._c('bb', 0x8892, vb[0]._id);
        for (const at of layout.attributes || []) {
          const f = GPU_VERTEX[at.format] || GPU_VERTEX.float32x4;
          gl._c('ea', at.shaderLocation, 1);
          const off = (vb[1] | 0) + (at.offset | 0);
          // uint/sint formats reach the shader as integers.
          if (f[1] !== 0x1406 && f[1] !== 0x140B && !f[2]) gl._c('aip', at.shaderLocation, f[0], f[1], layout.arrayStride | 0, off);
          else gl._c('ap', at.shaderLocation, f[0], f[1], f[2], layout.arrayStride | 0, off);
        }
      });
      // Fixed state.
      const prim = p.primitive || {};
      gl._c('en', 0xB44, prim.cullMode === 'front' || prim.cullMode === 'back' ? 1 : 0);
      if (prim.cullMode === 'front') gl._c('cf', 0x404); else if (prim.cullMode === 'back') gl._c('cf', 0x405);
      gl._c('ff', prim.frontFace === 'cw' ? 0x900 : 0x901);
      const dss = p.depthStencil;
      gl._c('en', 0xB71, dss ? 1 : 0);
      if (dss) { gl._c('df', GPU_COMPARE[dss.depthCompare] || 0x207); gl._c('dm', dss.depthWriteEnabled ? 1 : 0); }
      const tg = p.fragment && p.fragment.targets && p.fragment.targets[0];
      const b = tg && tg.blend;
      gl._c('en', 0xBE2, b ? 1 : 0);
      if (b) {
        const c = b.color || {}, a = b.alpha || {};
        gl._c('bf', GPU_BLEND[c.srcFactor] ?? 1, GPU_BLEND[c.dstFactor] ?? 0, GPU_BLEND[a.srcFactor] ?? 1, GPU_BLEND[a.dstFactor] ?? 0);
        gl._c('be', GPU_BLEND_OP[c.operation] || 0x8006, GPU_BLEND_OP[a.operation] || 0x8006);
      }
      const wm = tg && tg.writeMask !== undefined ? tg.writeMask : 15;
      gl._c('cm', wm & 1 ? 1 : 0, wm & 2 ? 1 : 0, wm & 4 ? 1 : 0, wm & 8 ? 1 : 0);
      // Bind groups: buffers and textures, then samplers (they set their textures' filters).
      let unit = 0;
      const samplers = [];
      groups.forEach((g, gi) => {
        if (!g) return;
        for (const e of g._entries) {
          const r = e.resource;
          if (r instanceof GPUSampler) { samplers.push([gi, e.binding, r]); continue; }
          const t = r instanceof GPUTextureView ? r.texture : r instanceof GPUTexture ? r : null;
          if (t) { t._upload(gl, unit); gl._c('gx', pipe._id, gi, e.binding, unit); unit++; continue; }
          const buf = r && (r.buffer || r);
          if (buf instanceof GPUBuffer) {
            buf._upload(gl);
            const off = r.offset | 0;
            gl._c('gb', pipe._id, gi, e.binding, buf._id, off, r.size === undefined ? buf.size - off : r.size | 0);
          }
        }
      });
      for (const [gi, bi, s] of samplers) gl._c('gs', pipe._id, gi, bi, ...s._f);
    };
    for (const c of pass.cmds) {
      switch (c[0]) {
        case 'pipe': pipe = c[1]; break;
        case 'vb': vbs[c[1]] = [c[2], c[3]]; break;
        case 'ib': index = [c[1], c[2], c[3]]; break;
        case 'bg': groups[c[1]] = c[2]; break;
        case 'vp': gl._c('vp', c[1] | 0, (H - c[2] - c[4]) | 0, c[3] | 0, c[4] | 0); gl._c('dr', numStr(c[5] || 0), numStr(c[6] === undefined ? 1 : c[6])); break;
        case 'sc': gl._c('en', 0xC11, 1); gl._c('sc', c[1] | 0, (H - c[2] - c[4]) | 0, c[3] | 0, c[4] | 0); break;
        case 'bc': gl._c('bc', ...c[1].map(numStr)); break;
        case 'draw': {
          if (!pipe) break;
          apply();
          const mode = GPU_TOPOLOGY[(pipe._d.primitive || {}).topology] ?? 4;
          for (let i = 0; i < c[2]; i++) gl._c('da', mode, c[3] | 0, c[1] | 0);
          break;
        }
        case 'drawi': {
          if (!pipe || !index) break;
          apply();
          const [buf, fmt, off] = index;
          buf._upload(gl);
          gl._c('bb', 0x8893, buf._id);
          const size = fmt === 'uint32' ? 4 : 2;
          const mode = GPU_TOPOLOGY[(pipe._d.primitive || {}).topology] ?? 4;
          for (let i = 0; i < c[2]; i++) gl._c('de', mode, c[1] | 0, size === 4 ? 0x1405 : 0x1403, (off | 0) + (c[3] | 0) * size);
          break;
        }
      }
    }
  }

  class GPUQueue extends GPUObjectBase {
    submit(buffers) {
      for (const b of buffers || []) for (const p of b._passes || []) { if (p.copy) p.copy(); else gpuRunPass(p); }
    }
    writeBuffer(buf, offset, data, dataOffset = 0, size) {
      const src = bytesOf(data);
      if (!src) return;
      const el = ArrayBuffer.isView(data) && data.BYTES_PER_ELEMENT ? data.BYTES_PER_ELEMENT : 1;
      const from = dataOffset * el, len = size === undefined ? src.byteLength - from : size * el;
      buf._bytes.set(src.subarray(from, from + len), offset);
      buf._ver++;
    }
    writeTexture(dst, data, layout, size) {
      const t = dst.texture, src = bytesOf(data);
      if (!t || !src) return;
      const [w, h] = gpuSize(size);
      const [ox, oy] = gpuOrigin(dst.origin);
      const bpr = (layout && layout.bytesPerRow) || w * 4, base = (layout && layout.offset) || 0;
      if (!t._px) t._px = new Uint8Array(t.width * t.height * 4);
      const bgra = t.format.startsWith('bgra');
      for (let y = 0; y < h && oy + y < t.height; y++) {
        for (let x = 0; x < w && ox + x < t.width; x++) {
          const s = base + y * bpr + x * 4, d = ((oy + y) * t.width + ox + x) * 4;
          t._px[d] = src[s + (bgra ? 2 : 0)]; t._px[d + 1] = src[s + 1]; t._px[d + 2] = src[s + (bgra ? 0 : 2)]; t._px[d + 3] = src[s + 3];
        }
      }
      t._pic = null; t._ver++;
    }
    copyExternalImageToTexture(src, dst) {
      const s = src.source || src, t = dst.texture;
      if (!t || !s) return;
      const addr = s.tagName === 'IMG' ? s.getAttribute('src') || '' : s.tagName === 'CANVAS' ? 'canvas:' + s._id : null;
      if (addr === null) return;
      t._pic = [addr, !!src.flipY]; t._ver++;
    }
    onSubmittedWorkDone() { return Promise.resolve(); }
  }

  const GPU_LIMITS = { maxTextureDimension1D: 4096, maxTextureDimension2D: 4096, maxTextureDimension3D: 256, maxTextureArrayLayers: 256,
    maxBindGroups: 4, maxBindingsPerBindGroup: 640, maxDynamicUniformBuffersPerPipelineLayout: 8, maxDynamicStorageBuffersPerPipelineLayout: 4,
    maxSampledTexturesPerShaderStage: 16, maxSamplersPerShaderStage: 16, maxStorageBuffersPerShaderStage: 8,
    maxStorageTexturesPerShaderStage: 4, maxUniformBuffersPerShaderStage: 12, maxUniformBufferBindingSize: 65536,
    maxStorageBufferBindingSize: 134217728, minUniformBufferOffsetAlignment: 256, minStorageBufferOffsetAlignment: 256,
    maxVertexBuffers: 8, maxBufferSize: 268435456, maxVertexAttributes: 16, maxVertexBufferArrayStride: 2048,
    maxInterStageShaderVariables: 16, maxColorAttachments: 8, maxColorAttachmentBytesPerSample: 32,
    maxComputeWorkgroupStorageSize: 16384, maxComputeInvocationsPerWorkgroup: 256, maxComputeWorkgroupSizeX: 256,
    maxComputeWorkgroupSizeY: 256, maxComputeWorkgroupSizeZ: 64, maxComputeWorkgroupsPerDimension: 65535 };
  class GPUDevice extends EventTarget {
    constructor(adapter) {
      super();
      this.adapter = adapter; this.features = new Set(); this.limits = Object.assign({}, GPU_LIMITS);
      this.queue = new GPUQueue(); this.lost = new Promise(() => {}); this.label = '';
    }
    createBuffer(d) { return new GPUBuffer(d); }
    createTexture(d) { return new GPUTexture(d); }
    createSampler(d) { return new GPUSampler(d); }
    createShaderModule(d) { return new GPUShaderModule(d); }
    createBindGroupLayout(d) { return new GPUBindGroupLayout(d); }
    createPipelineLayout(d) { return new GPUPipelineLayout(d); }
    createBindGroup(d) { return new GPUBindGroup(d); }
    createRenderPipeline(d) { return new GPURenderPipeline(d); }
    createRenderPipelineAsync(d) { return Promise.resolve(new GPURenderPipeline(d)); }
    createComputePipeline(d) { return { label: '', getBindGroupLayout: () => new GPUBindGroupLayout() }; }
    createComputePipelineAsync(d) { return Promise.resolve(this.createComputePipeline(d)); }
    createCommandEncoder(d) { return new GPUCommandEncoder(d); }
    createQuerySet(d) { return new GPUQuerySet(d); }
    pushErrorScope() {}
    popErrorScope() { return Promise.resolve(null); }
    destroy() {}
  }
  class GPUAdapter {
    constructor() {
      this.features = new Set(); this.limits = Object.assign({}, GPU_LIMITS); this.isFallbackAdapter = true;
      this.info = { vendor: 'SexOS', architecture: 'sex-gl', device: 'software', description: 'SexOS software renderer' };
    }
    requestDevice() { return Promise.resolve(new GPUDevice(this)); }
    requestAdapterInfo() { return Promise.resolve(this.info); }
  }
  class GPU {
    constructor() { this.wgslLanguageFeatures = new Set(); }
    requestAdapter() { return Promise.resolve(new GPUAdapter()); }
    getPreferredCanvasFormat() { return 'bgra8unorm'; }
  }
  class GPUCanvasContext {
    constructor(canvas) { this.canvas = canvas; this._gl = null; this._cfg = null; this._tex = null; }
    configure(cfg) {
      this._cfg = cfg;
      if (!this._gl) this._gl = new WebGLRenderingContext(this.canvas, { alpha: cfg.alphaMode === 'premultiplied', depth: true });
    }
    unconfigure() { this._cfg = null; }
    getConfiguration() { return this._cfg; }
    getCurrentTexture() {
      const c = this.canvas;
      if (!this._tex || this._tex.width !== c.width || this._tex.height !== c.height) {
        this._tex = new GPUTexture({ size: [c.width, c.height], format: (this._cfg && this._cfg.format) || 'bgra8unorm', usage: 16 }, this);
      }
      return this._tex;
    }
    _resize(w, h) { if (this._gl) this._gl._resize(w, h); }
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

  // fetch(): the browser loads the address after the script returns (from the same places
  // pages come from: built-in, @files, http) and answers with a 'fetched' message.
  const fetches = new Map();
  class Headers {
    constructor(init) {
      this._m = new Map();
      if (!init) return;
      if (init instanceof Headers) init._m.forEach((v, k) => this._m.set(k, v));
      else if (Array.isArray(init) || typeof init[Symbol.iterator] === 'function') for (const [k, v] of init) this.append(k, v);
      else for (const k of Object.keys(init)) this.append(k, init[k]);
    }
    get(k) { const v = this._m.get(String(k).toLowerCase()); return v === undefined ? null : v; }
    has(k) { return this._m.has(String(k).toLowerCase()); }
    set(k, v) { this._m.set(String(k).toLowerCase(), String(v)); }
    append(k, v) { k = String(k).toLowerCase(); const o = this._m.get(k); this._m.set(k, o === undefined ? String(v) : o + ', ' + v); }
    delete(k) { this._m.delete(String(k).toLowerCase()); }
    forEach(fn, self) { this._m.forEach((v, k) => fn.call(self, v, k, this)); }
    keys() { return this._m.keys(); }
    values() { return this._m.values(); }
    entries() { return this._m.entries(); }
    [Symbol.iterator]() { return this._m.entries(); }
    _lines() { let s = ''; this._m.forEach((v, k) => { s += k + ': ' + v + '\r\n'; }); return s; }
  }
  // Header lines from the browser ("name: value\r\n"...) as Headers.
  function parseHeaders(text) {
    const h = new Headers();
    for (const line of String(text || '').split(/\r?\n/)) { const i = line.indexOf(':'); if (i > 0) h.append(line.slice(0, i).trim(), line.slice(i + 1).trim()); }
    return h;
  }
  // The reply's final address and headers, from the browser.
  function replyHead(id, url) {
    const v = H(OP.FETCH_HEAD, '' + id);
    if (v === null) return { url, headers: new Headers() };
    const nl = v.indexOf('\n');
    return { url: v.slice(0, nl) || url, headers: parseHeaders(v.slice(nl + 1)) };
  }
  const STATUS_TEXT = { 200: 'OK', 201: 'Created', 204: 'No Content', 206: 'Partial Content', 301: 'Moved Permanently', 302: 'Found', 304: 'Not Modified', 400: 'Bad Request', 401: 'Unauthorized', 403: 'Forbidden', 404: 'Not Found', 500: 'Internal Server Error', 503: 'Service Unavailable' };
  class Response {
    constructor(id, status, url, length, headers) {
      this._id = id; this.status = status; this.ok = status >= 200 && status < 300; this.url = url;
      this.statusText = STATUS_TEXT[status] || ''; this.redirected = false; this.type = 'basic';
      this.headers = headers || new Headers({ 'content-length': String(length) }); this.bodyUsed = false; this.body = null;
    }
    // The body comes over once, as bytes or as text.
    _take(binary) {
      if (this.bodyUsed) return Promise.reject(new TypeError('Body has already been used'));
      this.bodyUsed = true;
      const v = H(binary ? OP.FETCH_BODY | 0x10000 : OP.FETCH_BODY, '' + this._id);
      return Promise.resolve(v === null ? (binary ? new ArrayBuffer(0) : '') : v);
    }
    arrayBuffer() { return this._take(true); }
    text() { return this._take(false); }
    json() { return this._take(false).then(JSON.parse); }
    blob() { const type = this.headers.get('content-type') || ''; return this._take(true).then(b => ({ size: b.byteLength, type, arrayBuffer: () => Promise.resolve(b) })); }
    clone() { return this; }
  }
  // A request body as the browser takes it: text, or bytes.
  function bodyBytes(body, headers) {
    if (body === undefined || body === null) return '';
    if (typeof body === 'string') { if (!headers.has('content-type')) headers.set('content-type', 'text/plain;charset=UTF-8'); return body; }
    if (body instanceof URLSearchParams) { if (!headers.has('content-type')) headers.set('content-type', 'application/x-www-form-urlencoded;charset=UTF-8'); return body.toString(); }
    if (body instanceof ArrayBuffer) return body;
    if (ArrayBuffer.isView(body)) return body.buffer.slice(body.byteOffset, body.byteOffset + body.byteLength);
    return String(body);
  }
  // One request to the browser; `done(status, length)` runs when it is loaded.
  function ask(url, method, headers, body, done) {
    // The body first: it may add a Content-Type the header lines must carry.
    const bytes = bodyBytes(body, headers) || '';
    const id = H(OP.FETCH, url, method, headers._lines(), bytes);
    if (id === null) return null;
    fetches.set(+id, done);
    return +id;
  }
  function fetch(input, init) {
    init = init || {};
    const isReq = input && typeof input === 'object' && 'url' in input;
    const url = String(isReq ? input.url : input);
    const method = String(init.method || (isReq && input.method) || 'GET').toUpperCase();
    const headers = new Headers(init.headers || (isReq && input.headers) || undefined);
    const body = init.body !== undefined ? init.body : (isReq ? input.body : undefined);
    return new Promise((resolve, reject) => {
      const id = ask(url, method, headers, body, (id, status, length) => {
        if (status === 0) { reject(new TypeError('Failed to fetch ' + url)); return; }
        const head = replyHead(id, url);
        const r = new Response(id, status, head.url, length, head.headers);
        r.redirected = head.url !== url && !url.startsWith('/') && url.indexOf('://') > 0;
        resolve(r);
      });
      if (id === null) reject(new TypeError('Failed to fetch ' + url));
    });
  }
  class Request {
    constructor(input, init) {
      init = init || {};
      this.url = String(input && input.url || input); this.method = String(init.method || 'GET').toUpperCase();
      this.headers = new Headers(init.headers); this.body = init.body === undefined ? null : init.body;
    }
  }
  function fetched(id, status, length) {
    const f = fetches.get(+id);
    if (!f) return;
    fetches.delete(+id);
    f(+id, status, length);
  }

  // XMLHttpRequest on the same loads as fetch. `async=false` is treated as async: the
  // browser loads between script turns, so a blocking load cannot be had.
  class XMLHttpRequest extends EventTarget {
    constructor() {
      super();
      this.readyState = 0; this.status = 0; this.statusText = ''; this.responseType = ''; this.response = null;
      this.responseURL = ''; this.timeout = 0; this.withCredentials = false; this.upload = new EventTarget();
      this._h = new Headers(); this._rh = new Headers(); this._id = null; this._text = '';
      this.onreadystatechange = this.onload = this.onerror = this.onloadend = this.onabort = this.ontimeout = this.onprogress = this.onloadstart = null;
    }
    get responseText() { if (this.responseType !== '' && this.responseType !== 'text') throw domError('responseText needs responseType text'); return this._text; }
    get responseXML() { return null; }
    // dispatchEvent also runs the `on<type>` property.
    _fire(type) { this.dispatchEvent(new Event(type)); }
    _state(n) { this.readyState = n; this._fire('readystatechange'); }
    open(method, url) { this._m = String(method || 'GET').toUpperCase(); this._u = String(url); this._h = new Headers(); this._id = null; this.status = 0; this._text = ''; this.response = null; this._state(1); }
    setRequestHeader(k, v) { if (this.readyState !== 1) throw domError('InvalidStateError'); this._h.append(k, v); }
    overrideMimeType() {}
    getResponseHeader(k) { return this._rh.get(k); }
    getAllResponseHeaders() { return this.readyState >= 2 ? this._rh._lines() : ''; }
    abort() { if (this._id !== null) fetches.delete(this._id); this._id = null; this.readyState = 0; this._fire('abort'); this._fire('loadend'); }
    send(body) {
      if (this.readyState !== 1) throw domError('InvalidStateError');
      this._fire('loadstart');
      this._id = ask(this._u, this._m, this._h, this._m === 'GET' || this._m === 'HEAD' ? null : body, (id, status, length) => {
        if (this._id !== id) return;
        this._id = null;
        if (status === 0) { this.readyState = 4; this._fire('readystatechange'); this._fire('error'); this._fire('loadend'); return; }
        const head = replyHead(id, this._u);
        this.status = status; this.statusText = STATUS_TEXT[status] || ''; this.responseURL = head.url; this._rh = head.headers;
        this._state(2); this._state(3);
        const t = this.responseType;
        const v = H(t === 'arraybuffer' || t === 'blob' ? OP.FETCH_BODY | 0x10000 : OP.FETCH_BODY, '' + id);
        if (t === 'arraybuffer') this.response = v || new ArrayBuffer(0);
        else if (t === 'blob') { const b = v || new ArrayBuffer(0); this.response = { size: b.byteLength, type: head.headers.get('content-type') || '', arrayBuffer: () => Promise.resolve(b) }; }
        else if (t === 'json') { try { this.response = JSON.parse(v || ''); } catch (e) { this.response = null; } }
        else { this._text = v || ''; this.response = this._text; }
        this._fire('progress');
        this._state(4);
        this._fire('load'); this._fire('loadend');
      });
      if (this._id === null) { const x = this; Promise.resolve().then(() => { x.readyState = 4; x._fire('readystatechange'); x._fire('error'); x._fire('loadend'); }); }
    }
  }
  Object.assign(XMLHttpRequest, { UNSENT: 0, OPENED: 1, HEADERS_RECEIVED: 2, LOADING: 3, DONE: 4 });

  // Query strings.
  class URLSearchParams {
    constructor(init) {
      this._l = [];
      if (init instanceof URLSearchParams) this._l = init._l.map(p => p.slice());
      else if (init && typeof init === 'object') for (const k of Object.keys(init)) this._l.push([k, String(init[k])]);
      else {
        const dec = t => { try { return decodeURIComponent(t.replace(/\+/g, ' ')); } catch (e) { return t; } };
        for (const part of String(init || '').replace(/^\?/, '').split('&')) {
          if (!part) continue;
          const i = part.indexOf('=');
          this._l.push(i < 0 ? [dec(part), ''] : [dec(part.slice(0, i)), dec(part.slice(i + 1))]);
        }
      }
    }
    get(k) { const p = this._l.find(p => p[0] === String(k)); return p ? p[1] : null; }
    getAll(k) { return this._l.filter(p => p[0] === String(k)).map(p => p[1]); }
    has(k) { return this._l.some(p => p[0] === String(k)); }
    set(k, v) {
      k = String(k); v = String(v);
      const i = this._l.findIndex(p => p[0] === k);
      if (i < 0) this._l.push([k, v]);
      else { this._l[i][1] = v; this._l = this._l.filter((p, j) => j <= i || p[0] !== k); }
      this._sync();
    }
    append(k, v) { this._l.push([String(k), String(v)]); this._sync(); }
    delete(k) { this._l = this._l.filter(p => p[0] !== String(k)); this._sync(); }
    forEach(fn) { for (const [k, v] of this._l) fn(v, k, this); }
    entries() { return this._l.map(p => p.slice())[Symbol.iterator](); }
    keys() { return this._l.map(p => p[0])[Symbol.iterator](); }
    values() { return this._l.map(p => p[1])[Symbol.iterator](); }
    [Symbol.iterator]() { return this.entries(); }
    sort() { this._l.sort((a, b) => a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0); this._sync(); }
    get size() { return this._l.length; }
    // A URL's own parameters write themselves back into its query.
    _sync() { if (this._u) { const q = this.toString(); this._u._q = q ? '?' + q : ''; } }
    toString() { const e = t => encodeURIComponent(t).replace(/%20/g, '+'); return this._l.map(([k, v]) => e(k) + '=' + e(v)).join('&'); }
  }
  // Addresses, split into their parts.
  class URL {
    constructor(u, base) {
      let href = String(u);
      if (!/^[a-z][a-z0-9+.-]*:/i.test(href)) {
        if (base === undefined) throw new TypeError('Invalid URL: ' + href);
        const b = String(base instanceof URL ? base.href : base);
        const origin = (/^[a-z]+:\/\/[^/?#]*/i.exec(b) || [''])[0];
        href = href.startsWith('//') ? b.replace(/:.*/, ':') + href : href.startsWith('/') ? origin + href
          // An empty, query-only or fragment-only reference keeps the base's path (and query).
          : href === '' || href.startsWith('#') ? b.replace(/#.*$/, '') + href
          : href.startsWith('?') ? b.replace(/[?#].*$/, '') + href
          : b.replace(/[?#].*$/, '').replace(/[^/]*$/, '') + href;
      }
      const m = /^([a-z][a-z0-9+.-]*:)(?:\/\/([^/?#]*))?([^?#]*)(\?[^#]*)?(#.*)?$/i.exec(href) || [];
      this.protocol = (m[1] || '').toLowerCase(); this.host = m[2] || ''; this.pathname = m[3] || (this.host ? '/' : '');
      // Dot segments go away: /a/b/../c is /a/c.
      if (this.pathname.startsWith('/')) {
        const out = [];
        const parts = this.pathname.split('/');
        parts.forEach((p, i) => { if (p === '..') { if (out.length > 1) out.pop(); } else if (p !== '.' || i === parts.length - 1) out.push(p === '.' ? '' : p); });
        this.pathname = out.join('/') || '/';
      }
      this.hash = m[5] && m[5] !== '#' ? m[5] : '';
      // searchParams and search are one query: changing either changes the other.
      this.searchParams = new URLSearchParams();
      this.searchParams._u = this;
      this.search = m[4] || '';
    }
    get search() { return this._q; }
    set search(v) {
      v = String(v);
      this._q = v && v !== '?' ? (v[0] === '?' ? v : '?' + v) : '';
      this.searchParams._l = new URLSearchParams(this._q)._l;
    }
    get hostname() { return this.host.replace(/:\d+$/, ''); }
    get port() { const m = /:(\d+)$/.exec(this.host); return m ? m[1] : ''; }
    get origin() { return this.host ? this.protocol + '//' + this.host : 'null'; }
    get href() { return this.protocol + (this.host || this.protocol === 'file:' ? '//' + this.host : '') + this.pathname + this.search + this.hash; }
    toString() { return this.href; }
    toJSON() { return this.href; }
  }

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
    navigator: { userAgent: 'Mozilla/5.0 (SexOS) Web/0.2', language: 'en', languages: ['en'], platform: 'SexOS', onLine: true, gpu: new GPU() },
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
    Event, UIEvent, MouseEvent, PointerEvent, WheelEvent, FocusEvent, KeyboardEvent, CustomEvent, EventTarget, Node, Text, CharacterData, Element, HTMLElement,
    HTMLAnchorElement, HTMLImageElement, HTMLInputElement, HTMLButtonElement, HTMLCanvasElement, HTMLTemplateElement,
    HTMLTextAreaElement, HTMLSelectElement, HTMLOptionElement, HTMLFormElement, HTMLLabelElement, FormData, InputEvent, SubmitEvent,
    Document, DocumentFragment, DOMTokenList, CanvasRenderingContext2D, CanvasGradient, CanvasPattern,
    WebGLRenderingContext, WebGL2RenderingContext, WebGLBuffer, WebGLTexture, WebGLProgram, WebGLShader, WebGLFramebuffer,
    WebGLRenderbuffer, WebGLVertexArrayObject, WebGLUniformLocation, WebGLActiveInfo, WebGLShaderPrecisionFormat,
    WebGLQuery, WebGLSampler, WebGLSync, WebGLTransformFeedback,
    GPU, GPUAdapter, GPUDevice, GPUBuffer, GPUTexture, GPUTextureView, GPUSampler, GPUShaderModule, GPUBindGroup,
    GPUBindGroupLayout, GPUPipelineLayout, GPURenderPipeline, GPUCommandEncoder, GPUCommandBuffer, GPURenderPassEncoder,
    GPUQueue, GPUCanvasContext, GPUBufferUsage, GPUTextureUsage, GPUShaderStage, GPUMapMode, GPUColorWrite,
    createImageBitmap: src => Promise.resolve(src),
    fetch, Request, Response, Headers, URL, URLSearchParams, XMLHttpRequest,
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
    try { return dispatch(msg); } finally { flushCanvases(); }
  }, configurable: false, writable: false, enumerable: false });
  // Mouse input: `mouse\tKIND\tNODE\tX\tY\tBUTTON\tBUTTONS\tSTEPS`, X and Y in
  // viewport pixels, NODE the element under the pointer (empty for none). Answers 1
  // when the page cancelled the default action (a click's link, a wheel's scroll).
  const mouse = { over: null, down: null, capture: null, lock: null, x: 0, y: 0, last: 0, lastAt: null, count: 0 };
  function fireMouse(el, type, Kind, init, bubbles) {
    const ev = new Kind(type, Object.assign({}, init, { bubbles: bubbles !== false, cancelable: bubbles !== false }));
    el.dispatchEvent(ev);
    return ev;
  }
  function chain(n) { const out = []; for (; n && n.nodeType === 1; n = n.parentNode) out.push(n); return out; }
  function mouseInput(f) {
    const kind = f[1], x = +f[3], y = +f[4], button = +f[5], buttons = +f[6], steps = +f[7];
    const hit = (f[2] && wrap(f[2])) || document.body || document.documentElement;
    if (!hit) return 0;
    const el = mouse.capture || mouse.lock || hit;
    const init = { clientX: x, clientY: y, button: kind === 'move' ? -1 : button, buttons, movementX: x - mouse.x, movementY: y - mouse.y, pointerId: 1, pointerType: 'mouse', isPrimary: true };
    if (kind !== 'leave') { mouse.x = x; mouse.y = y; }
    if (kind !== 'leave' && hit !== mouse.over) {
      const old = mouse.over, before = chain(old), after = chain(hit);
      if (old) { fireMouse(old, 'pointerout', PointerEvent, Object.assign({}, init, { relatedTarget: hit })); fireMouse(old, 'mouseout', MouseEvent, Object.assign({}, init, { relatedTarget: hit })); }
      for (const n of before) if (!after.includes(n)) { fireMouse(n, 'pointerleave', PointerEvent, init, false); fireMouse(n, 'mouseleave', MouseEvent, init, false); }
      mouse.over = hit;
      fireMouse(hit, 'pointerover', PointerEvent, Object.assign({}, init, { relatedTarget: old })); fireMouse(hit, 'mouseover', MouseEvent, Object.assign({}, init, { relatedTarget: old }));
      for (const n of after.reverse()) if (!before.includes(n)) { fireMouse(n, 'pointerenter', PointerEvent, init, false); fireMouse(n, 'mouseenter', MouseEvent, init, false); }
    }
    switch (kind) {
      case 'move': {
        const p = fireMouse(el, 'pointermove', PointerEvent, init);
        if (!p.defaultPrevented) fireMouse(el, 'mousemove', MouseEvent, init);
        return 0;
      }
      case 'down': {
        mouse.down = el;
        const p = fireMouse(el, 'pointerdown', PointerEvent, init);
        if (p.defaultPrevented) return 1;
        if (fireMouse(el, 'mousedown', MouseEvent, init).defaultPrevented) return 1;
        // Focus moves to the control (or link) pressed, and leaves when it is elsewhere.
        if (button === 0) setFocus(focusable(el));
        return 0;
      }
      case 'up': {
        const p = fireMouse(el, 'pointerup', PointerEvent, init);
        let kept = p.defaultPrevented;
        if (!kept) kept = fireMouse(el, 'mouseup', MouseEvent, init).defaultPrevented;
        const down = mouse.down; mouse.down = null;
        if (!buttons) mouse.capture = null;
        if (!down) return kept ? 1 : 0;
        // A click goes to the nearest element both the press and release were in.
        const target = chain(el).find(n => chain(down).includes(n));
        if (!target) return kept ? 1 : 0;
        if (button === 2) return fireMouse(target, 'contextmenu', MouseEvent, init).defaultPrevented ? 1 : 0;
        if (button !== 0) return fireMouse(target, 'auxclick', MouseEvent, init).defaultPrevented ? 1 : 0;
        const now = performance.now();
        mouse.count = mouse.lastAt === target && now - mouse.last < 500 ? mouse.count + 1 : 1;
        mouse.last = now; mouse.lastAt = target;
        const click = fireMouse(target, 'click', MouseEvent, Object.assign({}, init, { detail: mouse.count }));
        if (mouse.count === 2) fireMouse(target, 'dblclick', MouseEvent, Object.assign({}, init, { detail: 2 }));
        return click.defaultPrevented || kept ? 1 : 0;
      }
      case 'wheel': {
        const w = fireMouse(el, 'wheel', WheelEvent, Object.assign({}, init, { button: 0, deltaY: -steps * 100, deltaMode: 0 }));
        return w.defaultPrevented ? 1 : 0;
      }
      case 'leave': {
        const old = mouse.over; mouse.over = null;
        if (old) { fireMouse(old, 'pointerout', PointerEvent, init); fireMouse(old, 'mouseout', MouseEvent, init); for (const n of chain(old)) { fireMouse(n, 'pointerleave', PointerEvent, init, false); fireMouse(n, 'mouseleave', MouseEvent, init, false); } }
        return 0;
      }
    }
    return 0;
  }
  function dispatch(msg) {
    const f = String(msg).split('\t');
    switch (f[0]) {
      case 'mouse': return mouseInput(f);
      case 'flush': return 0;
      case 'tick': return runTimers(performance.now());
      case 'fetched': fetched(f[1], +f[2], +f[3]); return 0;
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
        const target = document._active || document.body || document;
        target.dispatchEvent(ev);
        if (ev.defaultPrevented) return 1;
        if (f[1] !== 'keydown') return edit.el && edit.el === document._active ? 1 : 0;
        if (edit.el && edit.el === document._active) return editKey(edit.el, ev.key, ev.ctrlKey) ? 1 : 0;
        const a = document._active;
        if (a && a.localName === 'select' && (ev.key === 'ArrowUp' || ev.key === 'ArrowDown')) {
          const n = a.options.length;
          if (n) { a.selectedIndex = Math.max(0, Math.min(n - 1, a.selectedIndex + (ev.key === 'ArrowUp' ? -1 : 1))); fire(a, 'input', InputEvent); fire(a, 'change'); }
          return 1;
        }
        if (a && (ev.key === ' ' || ev.key === 'Enter') && a.localName === 'input' && /^(checkbox|radio|submit|button|reset|image)$/.test(a.type)) { a.click(); return 1; }
        if (a && (ev.key === ' ' || ev.key === 'Enter') && a.localName === 'button') { a.click(); return 1; }
        return 0;
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
  }
})();
