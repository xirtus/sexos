// SPDX-License-Identifier: GPL-3.0-only
// IndexedDB for page scripts. Each database lives here while the page runs and is
// saved whole by the browser (op IDB) when a readwrite or versionchange transaction
// completes, so it lasts across runs in the browser's profile. Object stores with
// key paths, key generators and indexes (unique and multiEntry); get, getAll,
// getAllKeys, count, put, add, delete, clear and cursors over key ranges.
(function () {
  'use strict';
  const { H, OP, domError, addTimer, EventTarget, Event, report } = globalThis.__sx_idb;
  delete globalThis.__sx_idb;
  const later = fn => addTimer(fn, 0, [], false);

  // Keys: numbers < dates < strings < binary < arrays.
  function keyType(k) {
    if (typeof k === 'number') return isNaN(k) ? 0 : 1;
    if (k instanceof Date) return isNaN(k.getTime()) ? 0 : 2;
    if (typeof k === 'string') return 3;
    if (k instanceof ArrayBuffer || ArrayBuffer.isView(k)) return 4;
    if (Array.isArray(k)) return k.every(x => keyType(x)) ? 5 : 0;
    return 0;
  }
  const bytesOf = k => k instanceof ArrayBuffer ? new Uint8Array(k) : new Uint8Array(k.buffer, k.byteOffset, k.byteLength);
  function cmp(a, b) {
    const ta = keyType(a), tb = keyType(b);
    if (!ta || !tb) throw domError('The parameter is not a valid key.', 'DataError');
    if (ta !== tb) return ta < tb ? -1 : 1;
    switch (ta) {
      case 1: return a < b ? -1 : a > b ? 1 : 0;
      case 2: return cmp(a.getTime(), b.getTime());
      case 3: return a < b ? -1 : a > b ? 1 : 0;
      case 4: { const x = bytesOf(a), y = bytesOf(b); for (let i = 0; i < Math.min(x.length, y.length); i++) if (x[i] !== y[i]) return x[i] < y[i] ? -1 : 1; return cmp(x.length, y.length); }
      case 5: { for (let i = 0; i < Math.min(a.length, b.length); i++) { const c = cmp(a[i], b[i]); if (c) return c; } return cmp(a.length, b.length); }
    }
  }
  const validKey = k => { if (!keyType(k)) throw domError('The parameter is not a valid key.', 'DataError'); return k; };
  function evalPath(v, path) {
    if (Array.isArray(path)) { const out = []; for (const p of path) { const x = evalPath(v, p); if (x === undefined) return undefined; out.push(x); } return out; }
    if (path === '') return v;
    for (const part of path.split('.')) { if (v == null || typeof v !== 'object' && typeof v !== 'string') return undefined; v = part === 'length' && typeof v === 'string' ? v.length : v[part]; }
    return v;
  }
  function setPath(v, path, key) {
    const parts = path.split('.'); const last = parts.pop();
    for (const p of parts) { if (v[p] === undefined) v[p] = {}; v = v[p]; }
    v[last] = key;
  }

  // The saved form: JSON, with tags for what JSON can't hold.
  function enc(v) {
    if (v === undefined) return { $u: 1 };
    if (v === null || typeof v === 'boolean' || typeof v === 'string') return v;
    if (typeof v === 'number') return isFinite(v) ? (Object.is(v, -0) ? { $n: '-0' } : v) : { $n: String(v) };
    if (typeof v === 'bigint') return { $b: String(v) };
    if (v instanceof Date) return { $d: v.getTime() };
    if (v instanceof RegExp) return { $r: [v.source, v.flags] };
    if (v instanceof ArrayBuffer) return { $ab: Array.from(new Uint8Array(v)) };
    if (ArrayBuffer.isView(v)) return { $ta: v.constructor.name, v: Array.from(bytesOf(v)) };
    if (v instanceof Map) return { $m: Array.from(v, ([k, x]) => [enc(k), enc(x)]) };
    if (v instanceof Set) return { $s: Array.from(v, enc) };
    if (typeof Blob !== 'undefined' && v instanceof Blob) return { $blob: Array.from(v._b), type: v.type };
    if (Array.isArray(v)) return v.map(enc);
    if (typeof v === 'function' || typeof v === 'symbol') throw domError('The object could not be cloned.', 'DataCloneError');
    const o = {}; for (const k of Object.keys(v)) o[k] = enc(v[k]);
    return Object.keys(o).some(k => k[0] === '$') ? { $o: o } : o;
  }
  function dec(v) {
    if (v === null || typeof v !== 'object') return v;
    if (Array.isArray(v)) return v.map(dec);
    if ('$u' in v) return undefined;
    if ('$n' in v) return Number(v.$n);
    if ('$b' in v) return BigInt(v.$b);
    if ('$d' in v) return new Date(v.$d);
    if ('$r' in v) return new RegExp(v.$r[0], v.$r[1]);
    if ('$ab' in v) return new Uint8Array(v.$ab).buffer;
    if ('$ta' in v) { const T = globalThis[v.$ta] || Uint8Array; const b = new Uint8Array(v.v).buffer; return new T(b, 0, b.byteLength / (T.BYTES_PER_ELEMENT || 1)); }
    if ('$m' in v) return new Map(v.$m.map(([k, x]) => [dec(k), dec(x)]));
    if ('$s' in v) return new Set(v.$s.map(dec));
    if ('$blob' in v) return new Blob([new Uint8Array(v.$blob)], { type: v.type });
    const src = '$o' in v ? v.$o : v; const o = {};
    for (const k of Object.keys(src)) o[k] = dec(src[k]);
    return o;
  }
  const clone = v => dec(enc(v));

  // Databases of this page's origin, by name: { version, stores: { name: store } }.
  // A store: { keyPath, autoIncrement, next, records: [[key, value]] sorted, indexes }.
  const open = new Map();
  function loadDb(name) {
    if (open.has(name)) return open.get(name);
    const text = H(OP.IDB, 'load', name);
    let db = null;
    if (text) { try { db = dec(JSON.parse(text)); } catch (e) { db = null; } }
    if (db) open.set(name, db);
    return db;
  }
  function saveDb(name, db) {
    const r = H(OP.IDB, 'save', name, JSON.stringify(enc(db)));
    return r !== '!';
  }
  function find(records, key) {
    let lo = 0, hi = records.length;
    while (lo < hi) { const mid = (lo + hi) >> 1; if (cmp(records[mid][0], key) < 0) lo = mid + 1; else hi = mid; }
    return lo;
  }

  class IDBKeyRange {
    constructor(lower, upper, lowerOpen, upperOpen) { this.lower = lower; this.upper = upper; this.lowerOpen = !!lowerOpen; this.upperOpen = !!upperOpen; }
    includes(k) {
      validKey(k);
      if (this.lower !== undefined) { const c = cmp(k, this.lower); if (c < 0 || (c === 0 && this.lowerOpen)) return false; }
      if (this.upper !== undefined) { const c = cmp(k, this.upper); if (c > 0 || (c === 0 && this.upperOpen)) return false; }
      return true;
    }
    static only(k) { validKey(k); return new IDBKeyRange(k, k, false, false); }
    static lowerBound(k, open) { validKey(k); return new IDBKeyRange(k, undefined, open, true); }
    static upperBound(k, open) { validKey(k); return new IDBKeyRange(undefined, k, true, open); }
    static bound(a, b, ao, bo) { validKey(a); validKey(b); if (cmp(a, b) > 0) throw domError('The lower key is greater than the upper key.', 'DataError'); return new IDBKeyRange(a, b, ao, bo); }
  }
  const toRange = q => q === undefined || q === null ? null : q instanceof IDBKeyRange ? q : IDBKeyRange.only(q);
  const inRange = (range, k) => !range || range.includes(k);

  class DOMStringList extends Array {
    contains(s) { return this.includes(String(s)); }
    item(i) { return this[i] ?? null; }
  }
  const names = o => DOMStringList.from(Object.keys(o).sort());

  class IDBRequest extends EventTarget {
    constructor(source, tx) { super(); this.source = source; this.transaction = tx; this.readyState = 'pending'; this._result = undefined; this._error = null; this.onsuccess = this.onerror = null; }
    get result() { if (this.readyState !== 'done') throw domError('The request has not finished.', 'InvalidStateError'); return this._result; }
    get error() { if (this.readyState !== 'done') throw domError('The request has not finished.', 'InvalidStateError'); return this._error; }
  }
  class IDBOpenDBRequest extends IDBRequest { constructor() { super(null, null); this.onupgradeneeded = this.onblocked = null; } }
  class IDBVersionChangeEvent extends Event { constructor(type, init) { super(type, init); this.oldVersion = (init && init.oldVersion) || 0; this.newVersion = init && init.newVersion !== undefined ? init.newVersion : null; } }

  class IDBTransaction extends EventTarget {
    constructor(db, scope, mode) {
      super(); this.db = db; this.mode = mode; this._scope = scope; this._queue = []; this._done = false; this._active = true; this._aborted = false;
      this.error = null; this.oncomplete = this.onerror = this.onabort = null;
      this.durability = 'default';
      // A copy to go back to if it aborts.
      this._undo = mode === 'readonly' ? null : JSON.stringify(enc(db._data));
      later(() => this._step());
    }
    get objectStoreNames() { return DOMStringList.from(this._scope); }
    objectStore(name) {
      if (this._done) throw domError('The transaction has finished.', 'InvalidStateError');
      if (!this._scope.includes(name) || !this.db._data.stores[name]) throw domError("No objectStore named '" + name + "' in this transaction.", 'NotFoundError');
      return new IDBObjectStore(this, name);
    }
    _request(source, fn) {
      if (!this._active || this._done) throw domError('The transaction is not active.', 'TransactionInactiveError');
      const r = new IDBRequest(source, this);
      this._queue.push([r, fn]);
      return r;
    }
    _writable() { if (this.mode === 'readonly') throw domError('The transaction is read-only.', 'ReadOnlyError'); }
    _step() {
      if (this._done) return;
      while (this._queue.length) {
        const [r, fn] = this._queue.shift();
        try { r._result = fn(); r.readyState = 'done'; this._active = true; r.dispatchEvent(new Event('success')); }
        catch (e) {
          r._error = e; r.readyState = 'done'; this._active = true;
          const ev = new Event('error', { bubbles: true, cancelable: true });
          r.dispatchEvent(ev);
          if (!ev.defaultPrevented) { this.error = e; this.abort(); return; }
        }
        this._active = false;
      }
      // Requests made in the callbacks run next; with none, the transaction commits.
      later(() => { if (this._queue.length) this._step(); else this._commit(); });
    }
    _commit() {
      if (this._done) return;
      this._done = true;
      if (this.mode !== 'readonly' && !saveDb(this.db.name, this.db._data)) {
        this.error = domError('The quota has been exceeded.', 'QuotaExceededError');
        this._done = false; this.abort(); return;
      }
      this.dispatchEvent(new Event('complete'));
      if (this._after) this._after();
    }
    abort() {
      if (this._done && !this._queue) throw domError('The transaction has finished.', 'InvalidStateError');
      this._done = true; this._aborted = true;
      if (this._undo) this.db._data = dec(JSON.parse(this._undo)), open.set(this.db.name, this.db._data);
      for (const [r] of this._queue) { r.readyState = 'done'; r._error = domError('The transaction was aborted.', 'AbortError'); r.dispatchEvent(new Event('error', { bubbles: true, cancelable: true })); }
      this._queue = [];
      later(() => { this.dispatchEvent(new Event('abort')); if (this._after) this._after(); });
    }
    commit() { this._active = false; }
  }

  class IDBIndex {
    constructor(store, name) { this.objectStore = store; this.name = name; }
    get _meta() { return this.objectStore._data.indexes[this.name]; }
    get keyPath() { return this._meta.keyPath; }
    get unique() { return this._meta.unique; }
    get multiEntry() { return this._meta.multiEntry; }
    // [indexKey, primaryKey, value] in index order.
    _entries(range) {
      const out = [], m = this._meta;
      for (const [pk, v] of this.objectStore._data.records) {
        let k = evalPath(v, m.keyPath);
        const ks = m.multiEntry && Array.isArray(k) ? k : [k];
        for (const x of ks) if (x !== undefined && keyType(x) && inRange(range, x)) out.push([x, pk, v]);
      }
      return out.sort((a, b) => cmp(a[0], b[0]) || cmp(a[1], b[1]));
    }
    get(q) { const range = toRange(q); return this.objectStore._tx._request(this, () => { const e = this._entries(range)[0]; return e ? clone(e[2]) : undefined; }); }
    getKey(q) { const range = toRange(q); return this.objectStore._tx._request(this, () => { const e = this._entries(range)[0]; return e ? e[1] : undefined; }); }
    getAll(q, n) { const range = toRange(q); return this.objectStore._tx._request(this, () => this._entries(range).slice(0, n || undefined).map(e => clone(e[2]))); }
    getAllKeys(q, n) { const range = toRange(q); return this.objectStore._tx._request(this, () => this._entries(range).slice(0, n || undefined).map(e => e[1])); }
    count(q) { const range = toRange(q); return this.objectStore._tx._request(this, () => this._entries(range).length); }
    openCursor(q, dir) { return cursor(this, toRange(q), dir, true, () => this._entries(null)); }
    openKeyCursor(q, dir) { return cursor(this, toRange(q), dir, false, () => this._entries(null)); }
  }

  class IDBObjectStore {
    constructor(tx, name) { this.transaction = tx; this._tx = tx; this.name = name; }
    get _data() { return this._tx.db._data.stores[this.name]; }
    get keyPath() { return this._data.keyPath; }
    get autoIncrement() { return this._data.autoIncrement; }
    get indexNames() { return names(this._data.indexes); }
    _put(value, key, noOverwrite) {
      this._tx._writable();
      const s = this._data;
      value = clone(value);
      if (s.keyPath !== null && key !== undefined) throw domError('The object store uses in-line keys and the key parameter was provided.', 'DataError');
      if (s.keyPath !== null) {
        key = evalPath(value, s.keyPath);
        if (key === undefined) {
          if (!s.autoIncrement) throw domError('Evaluating the object store key path did not yield a value.', 'DataError');
          key = s.next++;
          setPath(value, s.keyPath, key);
        }
      } else if (key === undefined) {
        if (!s.autoIncrement) throw domError('The object store uses out-of-line keys and has no key generator and the key parameter was not provided.', 'DataError');
        key = s.next++;
      }
      validKey(key);
      if (s.autoIncrement && typeof key === 'number' && key >= s.next) s.next = Math.floor(key) + 1;
      const i = find(s.records, key);
      const here = i < s.records.length && cmp(s.records[i][0], key) === 0;
      if (here && noOverwrite) throw domError('Key already exists in the object store.', 'ConstraintError');
      for (const [iname, m] of Object.entries(s.indexes)) {
        if (!m.unique) continue;
        const k = evalPath(value, m.keyPath);
        const ks = m.multiEntry && Array.isArray(k) ? k : [k];
        for (const [pk, v] of s.records) {
          if (cmp(pk, key) === 0) continue;
          const o = evalPath(v, m.keyPath), os = m.multiEntry && Array.isArray(o) ? o : [o];
          if (ks.some(x => x !== undefined && keyType(x) && os.some(y => y !== undefined && keyType(y) && cmp(x, y) === 0))) throw domError("Unable to add key to index '" + iname + "': at least one key does not satisfy the uniqueness requirements.", 'ConstraintError');
        }
      }
      if (here) s.records[i][1] = value; else s.records.splice(i, 0, [key, value]);
      return key;
    }
    put(value, key) { return this._tx._request(this, () => this._put(value, key, false)); }
    add(value, key) { return this._tx._request(this, () => this._put(value, key, true)); }
    get(q) { const range = toRange(q); return this._tx._request(this, () => { const r = this._data.records.find(([k]) => inRange(range, k)); return r ? clone(r[1]) : undefined; }); }
    getKey(q) { const range = toRange(q); return this._tx._request(this, () => { const r = this._data.records.find(([k]) => inRange(range, k)); return r ? r[0] : undefined; }); }
    getAll(q, n) { const range = toRange(q); return this._tx._request(this, () => this._data.records.filter(([k]) => inRange(range, k)).slice(0, n || undefined).map(r => clone(r[1]))); }
    getAllKeys(q, n) { const range = toRange(q); return this._tx._request(this, () => this._data.records.filter(([k]) => inRange(range, k)).slice(0, n || undefined).map(r => r[0])); }
    count(q) { const range = toRange(q); return this._tx._request(this, () => this._data.records.filter(([k]) => inRange(range, k)).length); }
    delete(q) { this._tx._writable(); const range = toRange(q); return this._tx._request(this, () => { this._data.records = this._data.records.filter(([k]) => !inRange(range, k)); }); }
    clear() { this._tx._writable(); return this._tx._request(this, () => { this._data.records = []; }); }
    index(name) {
      if (!this._data.indexes[name]) throw domError("No index named '" + name + "'.", 'NotFoundError');
      return new IDBIndex(this, name);
    }
    createIndex(name, keyPath, opts) {
      if (this._tx.mode !== 'versionchange') throw domError('Indexes are made in an upgrade.', 'InvalidStateError');
      if (this._data.indexes[name]) throw domError("An index named '" + name + "' already exists.", 'ConstraintError');
      this._data.indexes[name] = { keyPath, unique: !!(opts && opts.unique), multiEntry: !!(opts && opts.multiEntry) };
      return new IDBIndex(this, name);
    }
    deleteIndex(name) {
      if (this._tx.mode !== 'versionchange') throw domError('Indexes are removed in an upgrade.', 'InvalidStateError');
      if (!this._data.indexes[name]) throw domError("No index named '" + name + "'.", 'NotFoundError');
      delete this._data.indexes[name];
    }
    openCursor(q, dir) { return cursor(this, toRange(q), dir, true, () => this._data.records.map(([k, v]) => [k, k, v])); }
    openKeyCursor(q, dir) { return cursor(this, toRange(q), dir, false, () => this._data.records.map(([k, v]) => [k, k, v])); }
  }

  class IDBCursor {
    constructor(source, request, direction) { this.source = source; this.request = request; this.direction = direction; this.key = undefined; this.primaryKey = undefined; this._value = undefined; }
    get value() { return this._value; }
    continue(key) { this._go(1, key); }
    advance(n) { if (!(n > 0)) throw new TypeError('advance needs a positive count'); this._go(n); }
    update(v) { const store = this.source instanceof IDBIndex ? this.source.objectStore : this.source; return store.put(v, store.keyPath === null ? this.primaryKey : undefined); }
    delete() { const store = this.source instanceof IDBIndex ? this.source.objectStore : this.source; return store.delete(this.primaryKey); }
  }
  class IDBCursorWithValue extends IDBCursor {}
  // A cursor walks `list()` ([key, primaryKey, value], ascending) in `dir`, within range.
  function cursor(source, range, dir, withValue, list) {
    dir = dir || 'next';
    const store = source instanceof IDBIndex ? source.objectStore : source;
    const tx = store._tx;
    let req;
    const c = new (withValue ? IDBCursorWithValue : IDBCursor)(source, null, dir);
    let last = undefined, lastPk = undefined;
    const reverse = dir.startsWith('prev'), unique = dir.endsWith('unique');
    const move = (count, want) => {
      let items = list().filter(e => inRange(range, e[0]));
      if (reverse) items.reverse();
      const after = e => {
        if (last === undefined) return true;
        const c1 = cmp(e[0], last);
        if (unique) return reverse ? c1 < 0 : c1 > 0;
        if (c1 !== 0) return reverse ? c1 < 0 : c1 > 0;
        const c2 = cmp(e[1], lastPk);
        return reverse ? c2 < 0 : c2 > 0;
      };
      items = items.filter(after);
      if (want !== undefined) items = items.filter(e => reverse ? cmp(e[0], want) <= 0 : cmp(e[0], want) >= 0);
      if (unique) { const seen = []; items = items.filter(e => { if (seen.length && cmp(seen[seen.length - 1], e[0]) === 0) return false; seen.push(e[0]); return true; }); }
      const e = items[count - 1];
      if (!e) { c.key = c.primaryKey = c._value = undefined; return null; }
      last = e[0]; lastPk = e[1];
      c.key = e[0]; c.primaryKey = e[1]; c._value = withValue ? clone(e[2]) : undefined;
      return c;
    };
    req = tx._request(source, () => move(1));
    c.request = req;
    c._go = (n, key) => {
      if (key !== undefined) validKey(key);
      req.readyState = 'pending';
      tx._queue.push([req, () => move(n, key)]);
    };
    return req;
  }

  class IDBDatabase extends EventTarget {
    constructor(name, data) { super(); this.name = name; this._data = data; this._closed = false; this.onversionchange = this.onclose = this.onabort = this.onerror = null; }
    get version() { return this._data.version; }
    get objectStoreNames() { return names(this._data.stores); }
    transaction(scope, mode) {
      if (this._closed) throw domError('The database connection is closing.', 'InvalidStateError');
      if (this._upgrade && !this._upgrade._done) throw domError('An upgrade is running.', 'InvalidStateError');
      const list = (typeof scope === 'string' ? [scope] : Array.from(scope)).map(String);
      if (!list.length) throw domError('No object stores named.', 'InvalidAccessError');
      for (const n of list) if (!this._data.stores[n]) throw domError("No objectStore named '" + n + "'.", 'NotFoundError');
      mode = mode || 'readonly';
      if (mode !== 'readonly' && mode !== 'readwrite') throw new TypeError('Bad transaction mode');
      return new IDBTransaction(this, list, mode);
    }
    createObjectStore(name, opts) {
      if (!this._upgrade || this._upgrade._done) throw domError('Object stores are made in an upgrade.', 'InvalidStateError');
      name = String(name);
      if (this._data.stores[name]) throw domError("An object store named '" + name + "' already exists.", 'ConstraintError');
      const keyPath = opts && opts.keyPath !== undefined && opts.keyPath !== null ? opts.keyPath : null;
      const autoIncrement = !!(opts && opts.autoIncrement);
      if (autoIncrement && (keyPath === '' || Array.isArray(keyPath))) throw domError('An autoIncrement store needs a simple key path.', 'InvalidAccessError');
      this._data.stores[name] = { keyPath, autoIncrement, next: 1, records: [], indexes: {} };
      this._upgrade._scope.push(name);
      return new IDBObjectStore(this._upgrade, name);
    }
    deleteObjectStore(name) {
      if (!this._upgrade || this._upgrade._done) throw domError('Object stores are removed in an upgrade.', 'InvalidStateError');
      if (!this._data.stores[name]) throw domError("No objectStore named '" + name + "'.", 'NotFoundError');
      delete this._data.stores[name];
    }
    close() { this._closed = true; }
  }

  const factory = {
    open(name, version) {
      name = String(name);
      if (version !== undefined && !(Number.isInteger(+version) && +version >= 1)) throw new TypeError('The version must be a whole number of at least 1.');
      const req = new IDBOpenDBRequest();
      later(() => {
        let data = loadDb(name);
        const old = data ? data.version : 0;
        const want = version === undefined ? (old || 1) : +version;
        const finish = (db, err) => {
          req.readyState = 'done';
          if (err) { req._error = err; req.dispatchEvent(new Event('error', { bubbles: true, cancelable: true })); return; }
          req._result = db; req.transaction = null;
          req.dispatchEvent(new Event('success'));
        };
        if (want < old) return finish(null, domError('The requested version is less than the existing version.', 'VersionError'));
        if (!data) data = { version: 0, stores: {} };
        open.set(name, data);
        const db = new IDBDatabase(name, data);
        if (want === old) return finish(db);
        data.version = want;
        const tx = new IDBTransaction(db, Object.keys(data.stores), 'versionchange');
        tx._undo = JSON.stringify(enc(Object.assign({}, data, { version: old })));
        db._upgrade = tx;
        req.transaction = tx; req._result = db; req.readyState = 'done';
        tx._after = () => {
          if (tx._aborted) { if (!old) { open.delete(name); H(OP.IDB, 'delete', name); } finish(null, tx.error || domError('The upgrade was aborted.', 'AbortError')); }
          else finish(db);
        };
        tx._active = true;
        try { req.dispatchEvent(new IDBVersionChangeEvent('upgradeneeded', { oldVersion: old, newVersion: want })); } catch (e) { report(e); }
        tx._active = false;
      });
      return req;
    },
    deleteDatabase(name) {
      name = String(name);
      const req = new IDBOpenDBRequest();
      later(() => {
        const data = loadDb(name);
        open.delete(name);
        H(OP.IDB, 'delete', name);
        req.readyState = 'done'; req._result = undefined;
        req.dispatchEvent(new IDBVersionChangeEvent('success', { oldVersion: data ? data.version : 0, newVersion: null }));
      });
      return req;
    },
    databases() {
      const list = (H(OP.IDB, 'list') || '').split('\n').filter(Boolean);
      return Promise.resolve(list.map(name => ({ name, version: (loadDb(name) || { version: 0 }).version })));
    },
    cmp(a, b) { return cmp(a, b); },
  };

  Object.assign(globalThis, { indexedDB: factory, IDBKeyRange, IDBRequest, IDBOpenDBRequest, IDBTransaction, IDBDatabase, IDBObjectStore, IDBIndex, IDBCursor, IDBCursorWithValue, IDBVersionChangeEvent, DOMStringList });
})();
