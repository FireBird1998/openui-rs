// Installed on RuntimeSession by the package guard in both wasm-bindgen targets.
// These originals are called only inside openuiCall, after argument admission.
const openuiSample = RuntimeSession.prototype.sampleQueryClock;
const openuiResumeAt = RuntimeSession.prototype.resumeQueriesAt;
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

// Bound the JS traversal too: shared subtrees can expand exponentially before
// the session gets a chance to admit the JSON. Charge each visit, not each object.
const openuiTransportBudget = 8 * 1024 * 1024;
function openuiToolJson(value) {
  let room = openuiTransportBudget;
  const depth = new WeakMap();
  return openuiStringify(value, function (key, item) {
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
    if (typeof item === 'number' && !Number.isFinite(item)) {
      throw new TypeError(`Tool result contains ${item}, which JSON cannot represent`);
    }
    if (typeof item === 'bigint') throw new TypeError('Tool result contains a BigInt, which JSON cannot represent');
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
    this.generation = {};
  }

  complete(ticket, generation, tag, value) {
    this.pending.delete(ticket);
    ticket.deliver = null;
    try {
      if (!ticket.call.pending) return;
      // Serializing a result and reporting a binding error can also run app
      // code. Reserve the next turn before either can cause another dispatch.
      this.deferDrain();
      const outcome = openuiToolOutcome(tag, value);
      // Reading the provider's value can run arbitrary application code.
      if (this.generation !== generation || !ticket.call.pending) return;
      try { openuiOwned.get('sampleQueryClock').call(this.session, openuiNow()); }
      catch (error) { this.report(error); }
      if (this.generation === generation && ticket.call.pending) {
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
      ticket.call.free();
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
    this.generation = {};
    this.arm(undefined);
    clearTimeout(this.deferred);
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
    clearTimeout(this.timer);
    this.due = due;
    if (due === undefined) return;
    this.timer = setTimeout(() => {
      if (this.due !== due) return;
      this.due = undefined;
      try {
        openuiOwned.get('advanceQueryClock').call(this.session, openuiNow());
        this.drain(true);
      } catch (error) { this.report(error); }
    }, Math.max(0, Number(due - openuiNow())));
  }

  deferDrain() {
    if (this.deferred !== undefined) return;
    this.deferred = setTimeout(() => {
      this.deferred = undefined;
      try { this.drain(true); }
      catch (error) { this.report(error); }
    }, 0);
  }

  drain(hostTurn = false) {
    if (this.deferred !== undefined) return;
    if ((this.delivering || (!hostTurn && this.pending.size && this.providerMicrotasks)) &&
        openuiDrivers.get(this.session) === this) {
      this.deferDrain();
      return;
    }
    const generation = this.generation;
    const update = openuiOwned.get('takeQueryUpdate').call(this.session);
    this.delivering = true;
    try {
      this.arm(update.nextRefreshDueMs);
      const count = update.callCount;
      if (count || update.changed) {
        for (const ticket of this.pending) {
          if (ticket.call.pending) continue;
          ticket.deliver = null;
          ticket.call.free();
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
      this.delivering = false;
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
      if (key === 'detachToolProvider') driver.drain();
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
