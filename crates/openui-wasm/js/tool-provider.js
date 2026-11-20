// Installed on RuntimeSession by the package guard in both wasm-bindgen targets.
// These originals are called only inside openuiCall, after argument admission.
const openuiSample = RuntimeSession.prototype.sampleQueryClock;
const openuiResumeAt = RuntimeSession.prototype.resumeQueriesAt;
// Keep request compaction private to receipts claimed by the automatic driver.
const openuiReleaseRequest = WantedToolCall.prototype.__releaseRequest;
if (typeof openuiReleaseRequest !== 'function') throw new Error('missing tool receipt helper');
delete WantedToolCall.prototype.__releaseRequest;
const openuiRawDraws = new WeakMap();
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
// string is charged like a string. Other realms' built-ins are not seen.
const openuiOwnGetter = (prototype, key) => Object.getOwnPropertyDescriptor(prototype, key).get;
const openuiTypedArray = Object.getPrototypeOf(Uint8Array.prototype);
// The typed-array tag getter returns undefined, rather than throwing, for
// anything else; this brand throws like every other entry's.
const openuiTypedTag = openuiOwnGetter(openuiTypedArray, Symbol.toStringTag);
function openuiTypedBrand() {
  const tag = openuiApply(openuiTypedTag, this, []);
  if (tag === undefined) throw new TypeError('not a typed array');
  return tag;
}
const openuiStringValue = String.prototype.valueOf;
const openuiObjectPrototype = Object.prototype;
const openuiGetPrototype = Object.getPrototypeOf;
const openuiFieldOf = Object.getOwnPropertyDescriptor;
const openuiDateToJson = Date.prototype.toJSON;
const openuiIsView = ArrayBuffer.isView;
const openuiIsRawJson = typeof JSON.isRawJSON === 'function' ? JSON.isRawJSON : undefined;
const openuiBuiltins = new Map([
  [Map.prototype, ['a Map', openuiOwnGetter(Map.prototype, 'size')]],
  [Set.prototype, ['a Set', openuiOwnGetter(Set.prototype, 'size')]],
  [ArrayBuffer.prototype, ['an ArrayBuffer', openuiOwnGetter(ArrayBuffer.prototype, 'byteLength')]],
  [DataView.prototype, ['a DataView', openuiOwnGetter(DataView.prototype, 'byteLength')]],
  [openuiTypedArray, ['typed', openuiTypedBrand]],
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

function openuiToolJson(value) {
  let room = openuiTransportBudget;
  const depth = new WeakMap();
  // Each prototype is classified once per result; each step is charged.
  const kinds = new Map();
  const builtinOf = item => {
    const first = openuiGetPrototype(item);
    if (first === null || first === openuiObjectPrototype) return undefined;
    if (kinds.has(first)) return kinds.get(first);
    let found;
    for (let proto = first; proto !== null && proto !== openuiObjectPrototype; proto = openuiGetPrototype(proto)) {
      if (--room < 0) throw openuiFailure('LimitExceeded', 'tool outcome size limit');
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
  return openuiStringify(value, function (key, item) {
    if (item !== null && typeof item === 'object') {
      // A boxed string is the one object JSON.stringify converts to text by
      // calling application code (its prototype's toString), so it is
      // recognized before anything else, whatever its prototype. A String
      // object's own length is always a read-only, hidden, permanent data
      // property; only an object that has one pays the throwing brand check.
      const length = openuiFieldOf(item, 'length');
      if (length && !length.writable && !length.enumerable && !length.configurable) {
        try { item = openuiApply(openuiStringValue, item, []); } catch { /* not a boxed string */ }
      }
    }
    if (item !== null && typeof item === 'object') {
      const builtin = builtinOf(item);
      const brand = builtin && brandOf(builtin, item);
      if (brand) {
        const [what] = builtin;
        if (what === 'number') {
          if (!Number.isFinite(brand.inner)) throw openuiCannot(String(brand.inner));
          item = brand.inner;
        } else if (what === 'string' || what === 'boolean') {
          item = brand.inner;
        } else if (what === 'typed') {
          throw openuiCannot((/^[AEIO]/.test(brand.inner) ? 'an ' : 'a ') + brand.inner);
        } else if (what === 'date') {
          // A date reaches the replacer only when its toJSON is not callable,
          // and JSON.stringify would then write {}.
          throw openuiCannot('a Date');
        } else {
          throw openuiCannot(what);
        }
      } else if (builtin?.[0] === 'a DataView' && openuiIsView(item)) {
        // A view over a detached buffer fails the byteLength brand.
        throw openuiCannot('a DataView');
      } else if (openuiIsRawJson?.(item)) {
        // Raw JSON is written as its text; charge that text.
        room -= openuiFieldOf(item, 'rawJSON').value.length;
      }
    } else if (item === null) {
      // Date's own toJSON turns an invalid date into null before this
      // replacer sees it (also when a replaced valueOf gives NaN). A data
      // property still holds the original.
      const field = openuiFieldOf(this, key);
      const original = field && openuiHasOwn(field, 'value') ? field.value : null;
      if (original !== null && typeof original === 'object') {
        const builtin = builtinOf(original);
        const brand = builtin?.[0] === 'date' && brandOf(builtin, original);
        if (brand && original.toJSON === openuiDateToJson) throw openuiCannot('an invalid Date');
      }
    }
    room -= 1 + key.length + (typeof item === 'string' ? item.length : 0);
    if (room < 0) throw openuiFailure('LimitExceeded', 'tool outcome size limit');
    if (item !== null && typeof item === 'object') {
      const level = (depth.get(this) ?? 0) + 1;
      if (level > 256) throw openuiFailure('LimitExceeded', 'tool outcome depth limit');
      depth.set(item, level);
    }
    if (typeof item === 'function') {
      throw new TypeError(item === value ? 'Tool returned a function, which JSON cannot represent'
        : 'Tool result contains a function, which JSON cannot represent');
    }
    if (typeof item === 'number' && !Number.isFinite(item)) throw openuiCannot(String(item));
    if (typeof item === 'bigint') throw openuiCannot('a BigInt');
    if (typeof item === 'symbol' && item !== value) throw openuiCannot('a symbol');
    return item;
  });
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

  notify(name, ...args) {
    const callback = this.options[name];
    if (!callback) return;
    // A failed error handler has no further recipient. Never reenter it.
    const failed = error => { if (name !== 'onError') this.report(error); };
    try {
      const returned = callback(...args);
      if (returned != null) Promise.resolve(returned).catch(failed);
    } catch (error) { failed(error); }
  }

  report(error) {
    let diagnostic;
    try { diagnostic = error instanceof Error ? error : new Error(openuiToolMessage(error)); }
    catch { diagnostic = new Error('tool provider driver callback failed'); }
    this.notify('onError', diagnostic);
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

  drain(hostTurn = false) {
    if (this.deferred !== undefined) return;
    const session = this.session;
    const busy = openuiGet(openuiBusy, session) > 0;
    if (openuiDrivers.get(session) !== this) {
      // A detached driver's last drain. Inside a callback or a completion
      // it is skipped; the session keeps the change for the next drain.
      if (busy) return;
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
      const error = update.evaluationError;
      const previous = this.evaluationError;
      this.evaluationError = error;
      if (error && this.generation === generation &&
          (error.kind !== previous?.kind || error.message !== previous?.message)) {
        this.report(openuiFailure(error.kind, error.message));
      }
      // Unlike evaluationError this flag is consumed by the drain.
      if (update.readerRoundsLimited && this.generation === generation) {
        this.report(openuiFailure('LimitExceeded', 'Query readers were still changing at the reader round limit.'));
      }
      const actions = update.actionCount;
      for (let i = 0n; i < actions && this.generation === generation; i++) {
        const action = update.actionAt(i);
        try { this.notify('onAction', action); }
        finally { action.free(); }
      }
      if (this.generation === generation && update.changed) {
        this.notify('onChange');
      }
    } finally {
      openuiLeave(session);
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
      const text = converted[0], streaming = converted[1];
      const parts = openuiParseJson(openuiApply(openuiReadBundle, this, [openuiResponseText(text, this), streaming]));
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
      const program = bundle.program;
      const sameParse = last && last.program === program && last.state.streaming === streaming && last.state.error === error;
      const output = openuiApply(openuiDrawResponse, this, [sameParse ? undefined : program, state,
        driver ? openuiNow() : undefined]);
      openuiPut(openuiRawDraws, this, {program, state});
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
    const driver = openuiDrivers.get(this);
    if (stop) {
      const result = openuiApply(guarded, this, args);
      if (!driver) {
        openuiManualProviders.delete(this);
        return result;
      }
      openuiDrivers.delete(this);
      driver.cancel(false);
      if (key === 'detachToolProvider') driver.drain(true);
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
