// Installed on RuntimeSession by the package guard in both wasm-bindgen targets.
// These originals are called only inside openuiCall, after argument admission.
const openuiSample = RuntimeSession.prototype.sampleQueryClock;
const openuiResumeAt = RuntimeSession.prototype.resumeQueriesAt;
// Keep request compaction private to receipts claimed by the automatic driver.
const openuiReleaseRequest = WantedToolCall.prototype.__releaseRequest;
if (typeof openuiReleaseRequest !== 'function') throw new Error('missing tool receipt helper');
delete WantedToolCall.prototype.__releaseRequest;
const openuiRawDraws = new WeakMap();
// The last raw response each session's reader accepted (ADR 0015). Only a
// text that continues it sends just its new units; the reader holds the rest.
const openuiResponseTexts = new WeakMap();
const openuiReadBundle = RuntimeSession.prototype.__readResponseBundle;
const openuiDrawResponse = RuntimeSession.prototype.__drawResponse;
if (typeof openuiReadBundle !== 'function' || typeof openuiDrawResponse !== 'function') {
  throw new Error('openui package guard: raw response helpers are missing');
}
// Only the captured functions participate in the compound operation.
delete RuntimeSession.prototype.__readResponseBundle;
delete RuntimeSession.prototype.__drawResponse;
const openuiDrivers = new WeakMap();
const openuiManualProviders = new WeakSet();
const openuiDriverWrites = new Set(['initialize', 'initializeField', 'setProgram', 'replaceProgram',
  'evaluate', 'set', 'setField', 'writeBinding', 'markBinding', 'registerFunction',
  'run', 'runLegacy', 'resume', 'retryFailedQueries', 'pauseQueries', 'resumeQueries']);
// Captured during installation, these calls still pass the package guard.
const openuiOwned = new Map(['sampleQueryClock', 'advanceQueryClock', 'resumeQueriesAt',
  'takeQueryUpdate', 'completeToolCall', 'attachToolProvider', 'attachToolProviderAt',
  'attachToolProviderWithMinimum'].map(key => [key, undefined]));
const openuiNow = () => BigInt(Math.floor(performance.now()));

// The yield between dispatch rounds is a host task, never a microtask
// (ADR 0014). Browsers stretch nested zero-delay timers to 4 ms, so prefer
// setImmediate where it exists (Bun, Node), then one MessageChannel message
// per callback (browsers), then a timer. The globals are read at each yield:
// fake timers that replace setImmediate (or, without it, setTimeout) govern
// the turn, but on Bun and Node faking only setTimeout does not.
//
// A source that throws falls back to a timer, the one source every host has;
// Bun's MessageChannel delivers ahead of due timers, so a server runtime
// never falls back to it. `refused` receives the error when the timer also
// throws, at once or, for setImmediate's second hop, from that hop's task.
const openuiChannelQueue = [];
let openuiChannel;
function openuiChannelTask(callback) {
  if (!openuiChannel) {
    // Keep the channel only once its handler is set.
    const channel = new MessageChannel();
    channel.port1.onmessage = () => { openuiChannelQueue.shift()?.(); };
    openuiChannel = channel;
    // An idle port must not keep a server runtime alive.
    openuiChannel.port1.unref?.();
    openuiChannel.port2.unref?.();
  }
  openuiChannelQueue.push(callback);
  try { openuiChannel.port2.postMessage(undefined); }
  catch (error) { openuiChannelQueue.pop(); throw error; }
}
function openuiPostTask(callback, refused) {
  try {
    if (typeof setImmediate === 'function') {
      // Posted from a timer callback, one immediate would run before the
      // loop's next timer pass. The second hop always follows a timer pass.
      setImmediate(() => {
        try { setImmediate(callback); return; } catch { /* fall back below */ }
        try { setTimeout(callback, 0); } catch (error) { refused(error); }
      });
      return;
    }
    if (typeof MessageChannel === 'function') {
      openuiChannelTask(callback);
      return;
    }
  } catch { /* fall back below */ }
  try { setTimeout(callback, 0); } catch (error) { refused(error); }
}

// Bound the JS traversal too: shared subtrees can expand exponentially before
// the session gets a chance to admit the JSON. Charge each visit, not each object.
const openuiTransportBudget = 8 * 1024 * 1024;

// Built-ins that JSON.stringify would empty or change in kind (ADR 0014), by
// prototype, each with a brand check that throws for any other object or
// returns undefined. Boxed primitives unbox to their own value, so a boxed
// string is charged like a string. Other realms' built-ins are not seen by
// this table.
const openuiOwnGetter = (prototype, key) => Object.getOwnPropertyDescriptor(prototype, key).get;
const openuiTypedArray = Object.getPrototypeOf(Uint8Array.prototype);
// Call the intrinsic directly on a confirmed view. It returns undefined for
// a DataView and never consults a provider's Symbol.toStringTag.
const openuiTypedTag = openuiOwnGetter(openuiTypedArray, Symbol.toStringTag);
const openuiStringValue = String.prototype.valueOf;
const openuiObjectPrototype = Object.prototype;
const openuiGetPrototype = Object.getPrototypeOf;
const openuiFieldOf = Object.getOwnPropertyDescriptor;
const openuiDateToJson = Date.prototype.toJSON;
const openuiIsView = ArrayBuffer.isView;
// Sentinels share the table's [name, intrinsic] layout.
const openuiTypedView = ['typed', undefined];
const openuiDataView = ['a DataView', undefined];
const openuiIsRawJson = typeof JSON.isRawJSON === 'function' ? JSON.isRawJSON : undefined;
const openuiBuiltins = new Map([
  [Map.prototype, ['a Map', openuiOwnGetter(Map.prototype, 'size')]],
  [Set.prototype, ['a Set', openuiOwnGetter(Set.prototype, 'size')]],
  [ArrayBuffer.prototype, ['an ArrayBuffer', openuiOwnGetter(ArrayBuffer.prototype, 'byteLength')]],
  [DataView.prototype, openuiDataView],
  [openuiTypedArray, openuiTypedView],
  [BigInt.prototype, ['a BigInt', BigInt.prototype.valueOf]],
  [Symbol.prototype, ['a symbol', Symbol.prototype.valueOf]],
  [Number.prototype, ['number', Number.prototype.valueOf]],
  [String.prototype, ['string', String.prototype.valueOf]],
  [Boolean.prototype, ['boolean', Boolean.prototype.valueOf]],
  [Date.prototype, ['date', Date.prototype.getTime]],
]);
if (typeof SharedArrayBuffer === 'function') {
  openuiBuiltins.set(SharedArrayBuffer.prototype,
    ['a SharedArrayBuffer', openuiOwnGetter(SharedArrayBuffer.prototype, 'byteLength')]);
}

function openuiCannot(what) {
  return new TypeError(`Tool result contains ${what}, which JSON cannot represent`);
}

// Fails for any view, typed array or DataView, by its internal kind.
function openuiCheckView(item) {
  const tag = openuiApply(openuiTypedTag, item, []);
  throw openuiCannot(tag === undefined ? 'a DataView' : (/^[AEIO]/.test(tag) ? 'an ' : 'a ') + tag);
}

const openuiBindEnumerable = Function.prototype.bind.bind(Object.prototype.propertyIsEnumerable);
const openuiSetPrototype = Object.setPrototypeOf;
// Provider hooks can change Array.prototype while the projection is running.
const openuiProjectionArray = () => openuiSetPrototype([], null);
const openuiJoin = Array.prototype.join;

function openuiToolJson(value) {
  let room = openuiTransportBudget;
  const path = openuiProjectionArray();
  let pathLength = 0;
  const charge = size => {
    room -= size;
    if (room < 0) throw openuiFailure('LimitExceeded', 'tool outcome size limit');
  };
  // Each prototype is classified once per result; each step is charged.
  const kinds = new Map();
  const builtinOf = item => {
    const first = openuiGetPrototype(item);
    if (first === null || first === openuiObjectPrototype) return undefined;
    if (kinds.has(first)) return kinds.get(first);
    let found;
    for (let proto = first; proto !== null && proto !== openuiObjectPrototype; proto = openuiGetPrototype(proto)) {
      charge(1);
      found = openuiBuiltins.get(proto);
      if (found) break;
    }
    kinds.set(first, found);
    return found;
  };
  // The object's own built-in value, or undefined when the brand is absent.
  const brandOf = (builtin, item) => {
    try { return { inner: openuiApply(builtin[1], item, []) }; }
    catch { return undefined; }
  };
  // Keep a bounded batch of tokens. Joining a full batch releases its small
  // strings even when traversal is still deep inside the first root child.
  const chunks = openuiProjectionArray(), tokens = openuiProjectionArray();
  const names = openuiProjectionArray(), positions = openuiProjectionArray();
  let used = 0, nameCount = 0;
  function flush() {
    // A string grows by exactly its two quotes if every code unit can be
    // copied as-is. A space between names keeps lone surrogates in adjacent
    // names from forming a pair. All these names have already been charged.
    if (nameCount) {
      names.length = nameCount;
      const sample = openuiApply(openuiJoin, names, [' ']);
      if (openuiStringify(sample).length !== sample.length + 2) {
        for (let i = 0; i < nameCount; i++) {
          tokens[positions[i]] = openuiApply(openuiSlice, openuiStringify(names[i]), [1, -1]);
        }
      }
      nameCount = 0;
    }
    tokens.length = used;
    chunks[chunks.length] = openuiApply(openuiJoin, tokens, ['']);
    used = 0;
  }
  // Object members supply a comma flag. Array entries and the root have no
  // property prefix. An omitted member never enters the buffer.
  function emit(key, comma, text) {
    if (used >= 4096) flush();
    if (comma !== undefined) {
      tokens[used++] = comma ? ',"' : '"';
      positions[nameCount] = used;
      names[nameCount++] = key;
      tokens[used++] = key;
      tokens[used++] = '":';
    }
    tokens[used++] = text;
  }
  let lastScalar, encodedScalar;
  let boxed;
  const holder = {toJSON: () => boxed}, noKeys = [];
  // Read the original and its conversion together. A stringify replacer sees
  // only the converted value, too late to distinguish a Date from a chosen null.
  function write(original, key, level, comma) {
    charge(1 + key.length);
    if (original !== null &&
        (typeof original === 'object' || typeof original === 'function' || typeof original === 'bigint')) {
      return writeObject(original, key, level, comma);
    }
    return scalar(original, key, level, comma);
  }
  function scalar(item, key, level, comma) {
    if (typeof item === 'number' && !Number.isFinite(item)) throw openuiCannot(String(item));
    if (typeof item === 'symbol' && level !== 0) throw openuiCannot('a symbol');
    if (item === undefined || typeof item === 'symbol') return false;
    if (typeof item === 'string') charge(item.length);
    if (item !== lastScalar) {
      encodedScalar = openuiStringify(item);
      lastScalar = item;
    }
    emit(key, comma, encodedScalar);
    return true;
  }
  function memberKeys(item, lengthField) {
    const keys = openuiOwnKeys(item);
    if (keys.length > room) throw openuiFailure('LimitExceeded', 'tool outcome size limit');
    // Snapshot enumerable keys before a value getter can edit a sibling.
    if (!keys.length) return keys;
    // Binding the native test once amortizes its cost on wider objects.
    const enumerable = keys.length >= 8 ? openuiBindEnumerable(item) : undefined;
    let count = 0;
    for (let i = 0; i < keys.length; i++) {
      const name = keys[i];
      if (typeof name !== 'string') continue;
      const visible = name === 'length' ? lengthField?.enumerable
        : enumerable ? enumerable(name) : openuiFieldOf(item, name)?.enumerable;
      if (visible) {
        if (count !== i) keys[count] = name;
        count++;
      }
    }
    charge(keys.length - count);
    if (count !== keys.length) keys.length = count;
    return keys;
  }
  function members(item, keys, level, original, toJSON) {
    let comma = false;
    for (let i = 0; i < keys.length; i++) {
      const name = keys[i];
      const child = item === original && name === 'toJSON' ? toJSON : item[name];
      if (write(child, name, level, comma)) comma = true;
    }
  }
  function writeObject(original, key, level, comma) {
    let item = original;
    const toJSON = original.toJSON;
    if (typeof toJSON === 'function') {
      item = openuiApply(toJSON, original, [key]);
      if (item === null && toJSON === openuiDateToJson) {
        const builtin = builtinOf(original);
        if (builtin?.[0] === 'date' && brandOf(builtin, original)) throw openuiCannot('an invalid Date');
      }
    }
    let lengthField;
    if (item !== null && typeof item === 'object') {
      // A boxed string has this mark even across realms and prototype changes.
      // Keep the descriptor for enumeration, which can itself run a Proxy trap.
      lengthField = openuiFieldOf(item, 'length');
      if (lengthField && !lengthField.writable && !lengthField.enumerable && !lengthField.configurable) {
        try { item = openuiApply(openuiStringValue, item, []); } catch { /* not a boxed string */ }
      }
    }
    if (item !== null && typeof item === 'object') {
      const builtin = builtinOf(item);
      if ((builtin === openuiTypedView || builtin === openuiDataView) && openuiIsView(item)) {
        openuiCheckView(item);
      }
      const brand = builtin?.[1] && brandOf(builtin, item);
      if (brand) {
        const what = builtin[0];
        if (what === 'number' || what === 'string' || what === 'boolean') item = brand.inner;
        else throw openuiCannot(what === 'date' ? 'a Date' : what);
      }
    }
    if (typeof item === 'function') {
      throw new TypeError(level === 0 ? 'Tool returned a function, which JSON cannot represent'
        : 'Tool result contains a function, which JSON cannot represent');
    }
    if (typeof item === 'bigint') throw openuiCannot('a BigInt');
    if (item === null || typeof item !== 'object') return scalar(item, key, level, comma);
    if (level >= 256) throw openuiFailure('LimitExceeded', 'tool outcome depth limit');
    if (openuiIsRawJson?.(item)) {
      const raw = openuiFieldOf(item, 'rawJSON').value;
      charge(raw.length);
      emit(key, comma, raw);
      return true;
    }
    // Reuse the common prefix of the last path. After a branch changes,
    // only the entries before this level are ancestors of the current item.
    let frame = path[level];
    if (frame === undefined) {
      frame = {item: undefined, firstRecord: undefined, lastRecord: undefined};
      path[level] = frame;
    }
    if (level >= pathLength || frame.item !== item) {
      for (let i = level - 1; i >= 0; i--) {
        if (path[i].item === item) throw new TypeError('Cannot serialize cyclic tool result');
      }
      frame.item = item;
      pathLength = level + 1;
    }
    if (openuiIsArray(item)) {
      const length = Math.trunc(+item.length);
      if (length > room) throw openuiFailure('LimitExceeded', 'tool outcome size limit');
      emit(key, comma, '[');
      for (let i = 0; i < length; i++) {
        if (i) tokens[used++] = ',';
        if (!write(item[i], openuiToString(i), level + 1)) emit(undefined, undefined, 'null');
      }
      tokens[used++] = ']';
    } else {
      // Stringify still unboxes a Number, Boolean or BigInt whose prototype
      // has changed. An empty property list lets the engine do just that
      // without visiting any children. The holder supplies the already
      // converted value so its toJSON is never looked up again.
      if (frame.firstRecord !== item && frame.lastRecord !== item) {
        boxed = item;
        const representation = openuiStringify(holder, noKeys);
        // Only a boxed Number, from any realm and on any chain, can unbox to
        // JSON null here. Stringify has already run its numeric conversion,
        // including any host valueOf.
        if (representation === 'null') throw openuiCannot('a Number');
        if (representation !== '{}') {
          emit(key, comma, representation);
          return true;
        }
        // A Date and a record can have the same chain and keys. Inspecting
        // Symbol.toStringTag, even by descriptor, would add Proxy traps.
        // ArrayBuffer.isView reads only the internal slot, so a typed array
        // or DataView fails by name on any chain, from any realm.
        if (openuiIsView(item)) openuiCheckView(item);
        // Empty-key unboxing depends on immutable internal slots. Remember
        // only this probe result. Each occurrence still reads its conversion and fields.
        if (frame.firstRecord === undefined) frame.firstRecord = item;
        else frame.lastRecord = item;
      }
      const keys = memberKeys(item, lengthField);
      if (!keys.length) {
        emit(key, comma, '{}');
        return true;
      }
      emit(key, comma, '{');
      members(item, keys, level + 1, original, toJSON);
      tokens[used++] = '}';
    }
    return true;
  }
  if (!write(value, '', 0)) return undefined;
  if (used) flush();
  return chunks.length === 1 ? chunks[0] : openuiApply(openuiJoin, chunks, ['']);
}

function openuiToolMessage(error) {
  try { return String(error instanceof Error ? error.message : error); }
  catch { return 'tool call failed'; }
}

// Only JS projection errors become failures here. Rust admits the final
// envelope, including the extracted MCP value and diagnostic text.
function openuiToolOutcome(tag, value) {
  if (tag === 'value' && value === undefined) return '{"undefined":true}';
  try {
    const json = openuiToolJson(tag === 'failed' ? openuiToolMessage(value) : value);
    if (json === undefined) {
      throw new TypeError(tag === 'mcpResponseJson' && value === undefined
        ? 'MCP callTool returned undefined instead of a response'
        : `Tool returned a ${typeof value}, which JSON cannot represent`);
    }
    return '{"' + tag + '":' + (tag === 'mcpResponseJson' ? openuiStringify(json) : json) + '}';
  } catch (error) {
    const message = openuiToolMessage(error);
    const text = message.length <= openuiTransportBudget ? message : 'tool outcome size limit';
    return '{"failed":' + openuiStringify(text) + '}';
  }
}

// Keep only a revocable delivery function on the promise. A provider may
// retain a never-settling promise after its session has been freed.
function openuiAwaitTool(promise, ticket, mcp) {
  Promise.resolve(promise).then(
    value => ticket.deliver?.(mcp ? 'mcpResponseJson' : 'value', value),
    error => ticket.deliver?.('failed', error),
  );
}

// A provider can keep its promise after the request leaves the cache. Its
// callback has no strong path to the driver, the receipt or the arguments.
function openuiReleasedDelivery(owner, ticket) {
  return () => {
    const driver = owner.deref();
    const call = driver && openuiGet(driver.released, ticket);
    if (!call) return;
    driver.released.delete(ticket);
    ticket.call = call;
    driver.complete(ticket, driver.generation, 'value', undefined);
  };
}

// Per session, not per driver, so that a driver replaced from provider code
// or a callback cannot escape either rule (ADR 0014):
// - Busy: a drain or a completion is on the stack. A drain asked for then
//   waits for a later task instead of running inside it.
// - Cancelled: calls still running were cancelled (pause, detach, free).
//   Their pending count no longer shows that provider code may be on the
//   stack, so drains wait for a later task until a driver's turn runs.
const openuiBusy = new WeakMap();
// A detached driver during its last drain, which a free or dispose made inside
// one of its callbacks must finish and stop (ADR 0014).
const openuiClosing = new WeakMap();
// Sessions the app has freed or disposed. Every driver the session ever had,
// current, closing or detached earlier, stops calling the app (ADR 0014).
const openuiEndedSessions = new WeakSet();
// One initial take, then at most one extra round per continuation slot for
// segments created during delivery. Read the Rust default lazily: the web
// package can be loaded before the WebAssembly module is initialized.
const openuiFlushes = new WeakMap();
let openuiDefaultContinuations;
function openuiFlushRounds(session) {
  const limits = openuiGet(openuiSessionLimits, session);
  if (limits) return limits.maxContinuations + 1n;
  if (openuiDefaultContinuations === undefined) {
    const defaults = new RuntimeLimits();
    try { openuiDefaultContinuations = defaults.maxContinuations; }
    finally { defaults.free(); }
  }
  return openuiDefaultContinuations + 1n;
}
const openuiCancelled = new WeakMap();
function openuiEnter(session) { openuiPut(openuiBusy, session, (openuiGet(openuiBusy, session) ?? 0) + 1); }
function openuiLeave(session) { openuiPut(openuiBusy, session, openuiGet(openuiBusy, session) - 1); }

function openuiStartTool(driver, call) {
  const generation = driver.generation;
  const ticket = { call, deliver: (tag, value) => driver.complete(ticket, generation, tag, value) };
  if (!driver.pending.size) {
    // Mark the checkpoint before invoking the provider: an await or then
    // can write to the session before the provider's own promise settles.
    driver.providerMicrotasks = false;
    queueMicrotask(() => { driver.providerMicrotasks = true; });
  }
  driver.pending.add(ticket);
  try {
    const current = driver.provider;
    const name = call.toolName;
    const json = call.argumentsJson;
    const args = json === undefined ? undefined : openuiParseJson(json);
    if (typeof current.callTool === 'function') {
      // Renderer reads the method again to invoke it with the client receiver.
      if (driver.generation !== generation) return;
      const method = current.callTool;
      if (driver.generation !== generation) return;
      openuiAwaitTool(openuiApply(method, current, [{ name, arguments: args }]), ticket, true);
    } else {
      if (driver.generation !== generation) return;
      const fn = current[name];
      if (driver.generation !== generation) return;
      if (!fn) {
        const names = Object.keys(current);
        queueMicrotask(() => ticket.deliver?.('availableTools', names));
      } else {
        openuiAwaitTool(fn(args), ticket, false);
      }
    }
  } catch (error) {
    queueMicrotask(() => ticket.deliver?.('failed', error));
  }
}

class OpenuiToolDriver {
  constructor(session, provider, options) {
    this.session = session;
    this.provider = provider;
    this.options = options;
    this.minimumMs = options.minimumMs ?? 1000n;
    this.pending = new Set();
    this.released = new WeakMap();
    this.owner = new WeakRef(this);
    this.generation = {};
  }

  complete(ticket, generation, tag, value) {
    this.pending.delete(ticket);
    ticket.deliver = null;
    let turn;
    const session = this.session;
    openuiEnter(session);
    try {
      if (this.generation !== generation) return;
      // Serializing a result and reporting a binding error can also run app
      // code. Reserve the next turn before either can cause another dispatch,
      // but post it afterwards: a timer that code schedules runs first.
      turn = this.reserveTurn();
      // Even a released query receipt triggers the manager's stale-reader
      // pass. Its payload has no current owner and must never run a getter.
      const outcome = ticket.call.pending ? openuiToolOutcome(tag, value) : '{"undefined":true}';
      // Reading the provider's value can run arbitrary application code.
      if (this.generation !== generation) return;
      try { openuiOwned.get('sampleQueryClock').call(this.session, openuiNow()); }
      catch (error) { this.report(error); }
      if (this.generation === generation) {
        openuiOwned.get('completeToolCall').call(this.session, ticket.call, outcome, true);
      }
    } catch (error) {
      let recoveryError;
      if (this.generation === generation && ticket.call.pending) {
        try {
          openuiOwned.get('completeToolCall').call(this.session, ticket.call,
            '{"failed":"Tool result delivery failed"}', true);
        } catch (fallbackError) {
          recoveryError = fallbackError;
          if (ticket.call.pending) RuntimeSession.prototype.detachToolProvider.call(this.session);
        }
      }
      this.report(error);
      if (recoveryError) this.report(recoveryError);
    } finally {
      openuiLeave(session);
      try { ticket.call.free(); }
      finally { this.postTurn(turn); }
    }
  }

  // After the app's free() or dispose() returns, no callback is called,
  // for any driver the session had (ADR 0014).
  stopped() {
    return openuiHas(openuiEndedSessions, this.session);
  }

  notify(name, options, ...args) {
    if (this.stopped()) return;
    const callback = (options ?? this.options)[name];
    if (!callback) return;
    // A failed error handler has no further recipient. Never reenter it.
    const failed = error => { if (!this.stopped() && name !== 'onError') this.report(error, options); };
    try {
      const returned = callback(...args);
      if (returned != null) Promise.resolve(returned).catch(failed);
    } catch (error) { failed(error); }
  }

  // The update a drain or a flush is delivering, and how far it got, so
  // that a free or dispose made inside one of its callbacks can hand over
  // the rest before it returns (ADR 0014).
  hold(update) {
    const hand = { update, index: 0n, options: undefined };
    this.inHand = hand;
    return hand;
  }

  release(hand) {
    if (this.inHand === hand) this.inHand = undefined;
  }

  // Whether this driver may still deliver: attached, and not freed or
  // disposed. Pausing or resuming from a callback does not stop delivery of
  // segments the session has already handed over.
  delivering() {
    if (this.stopped()) return false;
    const session = this.session;
    return openuiDrivers.get(session) === this || openuiGet(openuiClosing, session) === this;
  }

  // Each resumed action segment goes to onAction. Its failure, if any (a
  // later step's limit, or a pause that dropped its mutation: #117), first
  // goes to onError once, as the binding error it carries. Each delivery
  // advances the hand before its callback runs, so a nested flush neither
  // repeats it nor skips the next one.
  deliverHand(hand) {
    const count = hand.update.actionCount;
    while (hand.index < count && this.delivering()) {
      const action = hand.update.actionAt(hand.index);
      try {
        const options = hand.options ?? this.options;
        if (!hand.options) {
          hand.options = options;
          const failure = action.failure;
          if (failure) this.report(openuiFailure(failure.error.kind, failure.error.message), options);
          // A free inside onError delivered this segment's onAction and the rest.
          if (!this.delivering()) return;
        }
        hand.index++;
        hand.options = undefined;
        this.notify('onAction', options, action);
      } finally { action.free(); }
    }
  }

  // Before free, dispose or detach: hand over every action segment the
  // session has produced. First the rest of a drain or flush still on the
  // stack (a free made inside its callback), then what the session holds,
  // such as the failures of a pause whose drain waits for a task. Wanted
  // calls are released unstarted; nothing is armed or redrawn.
  // A drain's errors: the query evaluation error when it changes, and the
  // reader round cut, which each update reports once.
  reportEffects(update, current) {
    const error = update.evaluationError;
    const previous = this.evaluationError;
    this.evaluationError = error;
    if (error && current() &&
        (error.kind !== previous?.kind || error.message !== previous?.message)) {
      this.report(openuiFailure(error.kind, error.message));
    }
    if (update.readerRoundsLimited && current()) {
      this.report(openuiFailure('LimitExceeded', 'Query readers were still changing at the reader round limit.'));
    }
  }

  flushActions(take = true) {
    const session = this.session;
    const outer = openuiGet(openuiFlushes, session);
    const flush = outer ?? { rounds: openuiFlushRounds(session), taken: 0n, reported: false };
    if (!outer) openuiPut(openuiFlushes, session, flush);
    openuiEnter(session);
    try {
      if (this.inHand) this.deliverHand(this.inHand);
      if (!take) return;
      // A callback this delivers can produce more (a resume, a click and a
      // pause). Charge the take before delivery so a nested stop uses the
      // same remaining rounds and reports exhaustion only once.
      let more = true;
      while (more && this.delivering()) {
        if (flush.taken === flush.rounds) {
          if (!flush.reported) {
            flush.reported = true;
            this.report(openuiFailure('LimitExceeded',
              `free() stopped delivering after ${flush.rounds} rounds: callbacks kept producing action segments`));
          }
          return;
        }
        flush.taken++;
        const update = openuiOwned.get('takeQueryUpdate').call(session);
        const hand = this.hold(update);
        try {
          const count = update.callCount;
          for (let i = 0n; i < count; i++) update.callAt(i).free();
          // A drain would have reported these; the update is not taken twice.
          this.reportEffects(update, () => this.delivering());
          more = update.actionCount > 0n;
          this.deliverHand(hand);
        } finally {
          this.release(hand);
          update.free();
        }
      }
    } finally {
      openuiLeave(session);
      if (!outer) openuiFlushes.delete(session);
    }
  }

  report(error, options) {
    if (this.stopped()) return;
    let diagnostic;
    try { diagnostic = error instanceof Error ? error : new Error(openuiToolMessage(error)); }
    catch { diagnostic = new Error('tool provider driver callback failed'); }
    this.notify('onError', options, diagnostic);
  }

  cancel(paused) {
    // A reserved turn is the only thing holding the next round after a
    // completion, and cancelling drops it.
    if ((this.pending.size && this.providerMicrotasks) || this.deferred !== undefined ||
        openuiGet(openuiBusy, this.session) > 0) {
      openuiPut(openuiCancelled, this.session, true);
    }
    this.generation = {};
    this.released = new WeakMap();
    this.arm(undefined);
    this.deferred = undefined;
    for (const ticket of this.pending) {
      ticket.deliver = null;
      try {
        if (paused) openuiOwned.get('completeToolCall').call(this.session, ticket.call, '{"failed":"cancelled"}', true);
      } catch (error) {
        this.report(error);
      } finally {
        ticket.call.free();
      }
    }
    this.pending.clear();
  }

  arm(due) {
    if (this.due === due) return;
    try { clearTimeout(this.timer); }
    catch (error) { this.report(error); }
    this.timer = undefined;
    this.due = due;
    if (due === undefined) return;
    try {
      this.timer = setTimeout(() => {
        if (this.due !== due) return;
        this.due = undefined;
        try {
          openuiOwned.get('advanceQueryClock').call(this.session, openuiNow());
          this.drain(true);
        } catch (error) { this.report(error); }
      }, Math.max(0, Number(due - openuiNow())));
    } catch (error) {
      // No timer, no deadline: the next drain arms it again.
      this.due = undefined;
      this.timer = undefined;
      this.report(error);
    }
  }

  deferDrain() {
    this.postTurn(this.reserveTurn());
  }

  // While a turn is reserved every drain waits for it.
  reserveTurn() {
    if (this.deferred !== undefined) return undefined;
    return this.deferred = {};
  }

  postTurn(turn) {
    if (turn === undefined) return;
    // A cancelled turn still runs once; the token makes it do nothing.
    openuiPostTask(() => {
      this.refusalReported = false;
      if (this.deferred !== turn || openuiDrivers.get(this.session) !== this) return;
      this.deferred = undefined;
      try { this.drain(true); }
      catch (error) { this.report(error); }
    }, error => {
      // Every task source refused (ADR 0014). Report it once until a task
      // runs again, then release the turn so the next session operation
      // drains: a reservation with no task behind it would stop every later
      // drain. The turn stays reserved while onError runs, so a write it
      // makes waits for that operation instead of dispatching at once,
      // which would hold the event loop in a chain of microtasks.
      try {
        if (this.deferred === turn && !this.refusalReported) {
          this.refusalReported = true;
          this.report(error);
        }
      } finally {
        // The cancelled mark stays: clearing it let provider code that
        // pauses and resumes dispatch synchronously without bound.
        if (this.deferred === turn) this.deferred = undefined;
      }
    });
  }

  async postAfterBusy(turn) {
    // A fake timer can run a "task" on a provider's stack. A native await
    // leaves that stack even if the host fakes queueMicrotask as well.
    await undefined;
    if (this.deferred !== turn) return;
    openuiPut(openuiBusy, this.session, 0);
    this.postTurn(turn);
  }

  drain(hostTurn = false) {
    if (this.deferred !== undefined) return;
    const session = this.session;
    const busy = openuiGet(openuiBusy, session) > 0;
    let closing = false;
    if (openuiDrivers.get(session) !== this) {
      // A detached driver's last drain. Inside a callback or a completion
      // it is skipped; the session keeps the change for the next drain.
      if (busy || openuiGet(openuiClosing, session) !== undefined) return;
      // It delivers what it takes, as before, and a free inside one of its
      // callbacks finds it as the session's closing driver (ADR 0014).
      closing = true;
    } else if (busy && hostTurn) {
      this.postAfterBusy(this.reserveTurn());
      return;
    } else if (busy || (!hostTurn && (openuiGet(openuiCancelled, session) ||
        (this.pending.size && this.providerMicrotasks)))) {
      this.deferDrain();
      return;
    }
    // Only a current driver's turn ends the mark: a detached driver's final
    // drain runs right after the cancel that set it.
    if (hostTurn && openuiDrivers.get(session) === this) openuiCancelled.delete(session);
    const generation = this.generation;
    const update = openuiOwned.get('takeQueryUpdate').call(session);
    // Held before any callback, which could free the session (ADR 0014).
    const hand = this.hold(update);
    if (closing) openuiPut(openuiClosing, session, this);
    openuiEnter(session);
    try {
      this.arm(update.nextRefreshDueMs);
      const count = update.callCount;
      if (count || update.changed) {
        for (const ticket of this.pending) {
          if (ticket.call.pending) continue;
          const call = ticket.call;
          openuiCall(openuiNone, call, WantedToolCall.prototype, [], () => openuiApply(openuiReleaseRequest, call, []));
          openuiPut(this.released, ticket, call);
          ticket.call = null;
          ticket.deliver = openuiReleasedDelivery(this.owner, ticket);
          this.pending.delete(ticket);
        }
      }
      for (let i = 0n; i < count && this.generation === generation; i++) {
        const call = update.callAt(i);
        if (call.pending) openuiStartTool(this, call);
        else call.free();
      }
      this.reportEffects(update, () => this.generation === generation);
      this.deliverHand(hand);
      if (this.generation === generation && update.changed) {
        this.notify('onChange');
      }
    } finally {
      openuiLeave(session);
      if (closing && openuiGet(openuiClosing, session) === this) openuiClosing.delete(session);
      this.release(hand);
      update.free();
    }
  }
}

function openuiDriverOptions(input) {
  if (input === undefined) return undefined;
  if (input === null || typeof input !== 'object') throw openuiFailure('InvalidJson', 'expected tool provider options');
  const output = { __proto__: null, minimumMs: openuiOptionalLimit(input.minimumMs) };
  for (const key of ['onAction', 'onChange', 'onError']) {
    const callback = input[key];
    if (callback !== undefined && typeof callback !== 'function') throw openuiFailure('InvalidJson', 'expected a callback');
    output[key] = callback;
  }
  return output;
}

function openuiInstallDriver(prototype) {
  const responseSpec = openuiSpec([value => value === null ? '' : openuiString(value), openuiBoolean]);
  openuiDefine(prototype, 'setResponse', { value: function (...args) {
    const driver = openuiGet(openuiDrivers, this);
    const draw = openuiCall(responseSpec, this, prototype, args, converted => {
      const text = openuiResponseString(converted[0], this), streaming = converted[1];
      const accepted = openuiGet(openuiResponseTexts, this);
      // startsWith compares at memory speed; converting units to cross into
      // WebAssembly is the costly step, so only the new ones cross.
      const appends = accepted !== undefined && text.length >= accepted.length
        && openuiApply(openuiStartsWith, text, [accepted]);
      let parts = openuiParseJson(openuiApply(openuiReadBundle, this,
        [openuiResponseUnits(text, appends ? accepted.length : 0), appends ? accepted.length : undefined, streaming]));
      // The reader accepted another text than the one remembered here: send
      // the whole text once, which it reads afresh.
      if (parts.resend === true) {
        parts = openuiParseJson(openuiApply(openuiReadBundle, this, [openuiResponseUnits(text, 0), undefined, streaming]));
      }
      const bundle = parts.bundle;
      const name = openuiHasOwn(bundle.metadata, 'name') ? bundle.metadata.name : undefined;
      const error = openuiHasOwn(bundle, 'error') ? bundle.error : undefined;
      const revision = openuiHasOwn(bundle, 'scriptRevision') ? bundle.scriptRevision : undefined;
      const state = openuiResponseState({ __proto__: null, streaming, error, scriptRevision: revision,
        isBundle: bundle.isBundle, complete: bundle.complete, scripts: bundle.scripts }, this);
      let context = parts.context;
      if (context !== null) {
        try { context = openuiParseJson(context); } catch { /* A message can carry plain text. */ }
      }
      const last = openuiGet(openuiRawDraws, this);
      // The reader says whether the program differs from the last accepted one.
      const sameParse = last && !parts.programChanged && last.state.streaming === streaming && last.state.error === error;
      const output = openuiApply(openuiDrawResponse, this, [!sameParse, state,
        driver ? openuiNow() : undefined]);
      openuiPut(openuiRawDraws, this, {state});
      openuiPut(openuiResponseTexts, this, text);
      return {evaluation: openuiRemember(output), name, context, attributes: parts.attributes, error};
    });
    if (driver) driver.drain();
    return draw;
  } });
  const spec = openuiSpec([value => {
    if (value == null) return null;
    if (typeof value === 'object' || typeof value === 'function') return value;
    throw openuiFailure('InvalidJson', 'expected a tool provider or null');
  }, openuiDriverOptions]);
  openuiDefine(prototype, 'setToolProvider', { value: function (...args) {
    const [provider, options] = openuiCall(spec, this, prototype, args, converted => converted);
    const driver = openuiDrivers.get(this);
    if (provider === null) {
      this.detachToolProvider();
    } else if (driver) {
      if (options !== undefined) {
        if (options.minimumMs !== undefined && options.minimumMs !== driver.minimumMs) {
          throw openuiFailure('RefusedWrite', 'refresh minimum is fixed until detach');
        }
        driver.options = options;
      }
      driver.provider = provider;
    } else {
      if (openuiHas(openuiManualProviders, this)) throw openuiFailure('RefusedWrite', 'detach the manual tool provider first');
      openuiOwned.get('attachToolProviderAt').call(this, openuiNow(), options?.minimumMs ?? 1000n);
      const next = new OpenuiToolDriver(this, provider, options ?? {});
      openuiDrivers.set(this, next);
      next.drain();
    }
  } });
}

// A session that ended cannot produce further updates. Cancel outstanding
// deliveries outside the package guard so their receipts can be freed.
function openuiEndSession(session) {
  openuiAdd(openuiEndedSessions, session);
  openuiManualProviders.delete(session);
  const driver = openuiDrivers.get(session);
  if (driver) {
    openuiDrivers.delete(session);
    driver.cancel(false);
  }
}

function openuiDriverMember(key, spec, prototype, original) {
  // A successful low-level handoff takes ownership back from setResponse.
  const generated = original;
  if (['setProgram', 'replaceProgram', 'evaluate', 'free', 'dispose'].includes(key)) {
    original = function (...args) {
      const result = openuiApply(generated, this, args);
      if (key !== 'evaluate' || args[0] !== openuiGet(openuiRawDraws, this)?.state) {
        openuiPut(openuiRawDraws, this, undefined);
      }
      return result;
    };
  }
  const guarded = openuiMember(spec, prototype, original);
  if (openuiOwned.has(key)) {
    const attaches = key.startsWith('attachToolProvider');
    openuiOwned.set(key, guarded);
    return function (...args) {
      if (!openuiDrivers.has(this)) {
        const result = openuiApply(guarded, this, args);
        if (attaches) openuiAdd(openuiManualProviders, this);
        return result;
      }
      return openuiCall(openuiNone, this, prototype, [], () => {
        throw openuiFailure('RefusedWrite', `the attached tool provider driver owns ${key}`);
      });
    };
  }
  const stop = key === 'free' || key === 'dispose' || key === 'detachToolProvider';
  if (!stop && !openuiDriverWrites.has(key)) return guarded;
  return function (...args) {
    if (stop) openuiRefuseReentry();
    const driver = openuiGet(openuiDrivers, this);
    if (stop) {
      // Every segment the session produced reaches the app before it stops;
      // after free or dispose returns no callback runs (ADR 0014).
      // A driver attached inside a closing drain's callback: the closing one
      // finishes its update, the current one takes what is left.
      const closing = openuiGet(openuiClosing, this);
      if (closing && closing !== driver) {
        try { closing.flushActions(!driver); }
        catch (error) { closing.report(error); }
      }
      if (driver) {
        try { driver.flushActions(); }
        catch (error) { driver.report(error); }
      }
      const result = openuiApply(guarded, this, args);
      if (key !== 'detachToolProvider') {
        openuiEndSession(this);
        return result;
      }
      if (!driver) {
        openuiManualProviders.delete(this);
        return result;
      }
      openuiDrivers.delete(this);
      driver.cancel(false);
      driver.drain(true);
      return result;
    }
    if (!driver) return openuiApply(guarded, this, args);
    const result = openuiCall(spec, this, prototype, args, converted => {
      if (key === 'resumeQueries') return openuiApply(openuiResumeAt, this, [openuiNow()]);
      if (key !== 'pauseQueries') {
        openuiApply(openuiSample, this, [openuiNow()]);
      }
      return openuiApply(original, this, converted);
    });
    if (key === 'pauseQueries') driver.cancel(true);
    driver.drain();
    return result;
  };
}
