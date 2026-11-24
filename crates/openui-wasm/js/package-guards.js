// Appended inside both wasm-bindgen modules by scripts/wasm.sh, so it runs
// while the module loads, before application code can.
//
// Rules this guard enforces (ADR 0007, docs/wasm-runtime-binding-api.md):
// 1. Every public member of every generated class, string- or symbol-keyed,
//    statics included, is replaced by a guarded one; [Symbol.dispose] is the
//    guarded free. A member the surface table below does not describe stops
//    the module from loading, and so does a table entry the package lacks.
// 2. A guarded call refuses reentry, converts every argument to its primitive
//    type (or checks a minted, live capability) BEFORE the generated code reads
//    any pointer, and only then calls it. With primitive arguments the
//    generated conversions run no application code.
// 3. A freed object used in any way throws Disposed.
// 4. Generated code reaches nothing through an instance a host can redirect
//    (#211): the pointer field is an own accessor nothing can shadow, free
//    runs on the helper captured at load, and instance.constructor is the
//    guarded class. The private helpers _assertClass and __wrap run no host
//    code either (#217): they use the guard's own record and intrinsics
//    captured at load.
// Apart from registered host functions, the core calls no JavaScript while
// holding an exported object: errors and
// records come back as text and are built here after Rust has returned, with
// intrinsics captured at load. Generated private `__*` helpers work only
// inside a guarded call, and pointer fields cannot be overwritten.

// Intrinsics, captured before application code can replace them.
const openuiApply = Reflect.apply;
const openuiConstruct = Reflect.construct;
const openuiOwnKeys = Reflect.ownKeys;
const openuiDefine = Object.defineProperty;
const openuiCreate = Object.create;
const openuiDescriptor = Object.getOwnPropertyDescriptor;
const openuiPrototypeOf = Object.getPrototypeOf;
const openuiHasOwn = Object.hasOwn;
const openuiFreeze = Object.freeze;
const openuiIsArray = Array.isArray;
const openuiParseJson = JSON.parse;
const openuiStringify = JSON.stringify;
const openuiError = Error;
const openuiToString = String;
const openuiNumber = Number;
const openuiProxy = Proxy;
const openuiSetHas = WeakSet.prototype.has;
const openuiSetAdd = WeakSet.prototype.add;
const openuiMapGet = WeakMap.prototype.get;
const openuiMapSet = WeakMap.prototype.set;
const openuiSlice = String.prototype.slice;
const openuiIndexOf = String.prototype.indexOf;
const openuiCharCodeAt = String.prototype.charCodeAt;
const openuiStartsWith = String.prototype.startsWith;
const openuiFromCharCode = String.fromCharCode;
const openuiIsWellFormed = String.prototype.isWellFormed;
const openuiToWellFormed = String.prototype.toWellFormed;
const openuiDecode = TextDecoder.prototype.decode;
const openuiDecoder = new TextDecoder('utf-8', { ignoreBOM: true });
const openuiUint8Array = Uint8Array;
const openuiUint16Array = Uint16Array;
const openuiUint16Length = openuiDescriptor(openuiPrototypeOf(openuiUint16Array.prototype), 'length').get;
const openuiUint16Set = openuiUint16Array.prototype.set;
const openuiMemoryBuffer = openuiDescriptor(WebAssembly.Memory.prototype, 'buffer').get;
const openuiTableGet = WebAssembly.Table.prototype.get;
const openuiGlobalValue = openuiDescriptor(WebAssembly.Global.prototype, 'value');
const openuiRegister = typeof FinalizationRegistry === 'function' ? FinalizationRegistry.prototype.register : undefined;
const openuiUnregister = typeof FinalizationRegistry === 'function' ? FinalizationRegistry.prototype.unregister : undefined;
const openuiTableSet = WebAssembly.Table.prototype.set;
const openuiViewClass = DataView;
const openuiSetInt32 = DataView.prototype.setInt32;
const openuiDisposeSymbol = typeof Symbol.dispose === 'symbol' ? Symbol.dispose : undefined;

const openuiHas = (set, value) => openuiApply(openuiSetHas, set, [value]);
const openuiAdd = (set, value) => { openuiApply(openuiSetAdd, set, [value]); };
const openuiGet = (map, key) => openuiApply(openuiMapGet, map, [key]);
const openuiPut = (map, key, value) => { openuiApply(openuiMapSet, map, [key, value]); };
const openuiIsPrivate = key => typeof key === 'string' && openuiApply(openuiSlice, key, [0, 2]) === '__';

let openuiActive = false;
const openuiFreed = new WeakSet();
const openuiMinted = new WeakSet();
const openuiClassOf = new WeakMap();
const openuiPointers = new WeakMap();
const openuiOwnErrors = new WeakSet();

function openuiFailure(kind, message) {
  const error = new openuiError(message);
  // defineProperty, not assignment: an Object.prototype.kind setter never runs.
  openuiDefine(error, 'kind', { value: kind, writable: true, enumerable: true, configurable: true });
  openuiAdd(openuiOwnErrors, error);
  return error;
}
const openuiKinds = { __proto__: null, Disposed: true, ForeignHandle: true,
  StaleHandle: true, ConsumedHandle: true, EvictedHandle: true, InvalidHandle: true,
  InvalidToolCall: true, ReferenceConflict: true, InvalidMarker: true, InvalidJson: true, RefusedWrite: true, Evaluation: true,
  LimitExceeded: true, Worker: true, InvalidSchema: true, InvalidPromptSpec: true, Prompt: true,
  QueriesPaused: true, InvalidAttribute: true };
// Rust throws the string `openui:<Kind>\n<message>`. Anything else thrown by
// generated code is unexpected: report it as Worker without running accessors.
function openuiTyped(error) {
  if (error !== null && typeof error === 'object' && openuiHas(openuiOwnErrors, error)) return error;
  if (typeof error === 'string' && openuiApply(openuiSlice, error, [0, 7]) === 'openui:') {
    const newline = openuiApply(openuiIndexOf, error, ['\n']);
    const kind = newline > 7 ? openuiApply(openuiSlice, error, [7, newline]) : '';
    if (openuiKinds[kind] === true) return openuiFailure(kind, openuiApply(openuiSlice, error, [newline + 1]));
  }
  let message = 'unexpected failure in the WebAssembly binding';
  if (typeof error === 'string') message = error;
  else if (error !== null && typeof error === 'object') {
    const field = openuiDescriptor(error, 'message');
    if (field !== undefined && openuiHasOwn(field, 'value') && typeof field.value === 'string') message = field.value;
  }
  return openuiFailure('Worker', message);
}

// The generated helpers that run while Rust holds an object: the string cast
// an error is created with, and the externref read that hands it over.
// Rebuild them on captured intrinsics.
if (typeof getStringFromWasm0 !== 'function' || typeof takeFromExternrefTable0 !== 'function') {
  throw new openuiError('openui package guard: generated string helpers are missing');
}
getStringFromWasm0 = function (ptr, len) {
  const buffer = openuiApply(openuiMemoryBuffer, wasm.memory, []);
  return openuiApply(openuiDecode, openuiDecoder, [new openuiUint8Array(buffer, ptr >>> 0, len >>> 0)]);
};
takeFromExternrefTable0 = function (idx) {
  const value = openuiApply(openuiTableGet, wasm.__wbindgen_externrefs, [idx]);
  wasm.__externref_table_dealloc(idx);
  return value;
};

// The private helpers generated code calls inside a guarded call (#217). Host
// code must not run in them: _assertClass did `instance instanceof klass`,
// which walks the prototype chain and runs a Proxy prototype's getPrototypeOf
// trap, where the private statics (__wrap) are usable; __wrap looked up the
// global Object.create, and the registry's register and the pointer field
// assignment through the prototype chain, at each call. Class membership is
// the guard's own record of what it minted (openuiMinted, openuiClassOf),
// read with intrinsics captured at load; the generated message is kept.
if (typeof _assertClass !== 'function') {
  throw new openuiError('openui package guard: generated class check is missing');
}
_assertClass = function (instance, klass) {
  if (instance === null || typeof instance !== 'object' || !openuiHas(openuiMinted, instance)
    || openuiGet(openuiClassOf, instance) !== klass.prototype) {
    throw new openuiError(`expected instance of ${klass.name}`);
  }
};
// A generated class's __wrap, rebuilt: the prototype, Object.create and the
// registry's register are fixed at load, the pointer is defined as an own
// property (no setter on Object.prototype runs) and the object is minted, so
// the checks above admit it (run's retained token is such an object).
function openuiWrapper(klass, registry) {
  const prototype = klass.prototype;
  return function (ptr) {
    const obj = openuiCreate(prototype);
    openuiDefine(obj, '__wbg_ptr', { value: ptr, writable: true, enumerable: true, configurable: true });
    if (openuiRegister !== undefined) openuiApply(openuiRegister, registry, [obj, ptr, obj]);
    return openuiRemember(obj);
  };
}

// The generated __destroy_into_raw, rebuilt the same way: read the pointer,
// zero it, unregister, hand the pointer back, with the registry's unregister
// captured at load (free and the interrupted-instance release both run it
// inside a guarded call). The pointer goes through the own accessor, as in
// the generated code.
function openuiDestroyer(registry) {
  return function () {
    const ptr = this.__wbg_ptr;
    this.__wbg_ptr = 0;
    if (openuiUnregister !== undefined) openuiApply(openuiUnregister, registry, [this]);
    return ptr;
  };
}
// Two more generated helpers that run in a guarded call and looked up a
// prototype method on a live object: the externref table's set, and the
// DataView the imports write results through (only setInt32 is used).
if (typeof addToExternrefTable0 !== 'function' || typeof getDataViewMemory0 !== 'function') {
  throw new openuiError('openui package guard: generated table helpers are missing');
}
addToExternrefTable0 = function (obj) {
  const idx = wasm.__externref_table_alloc();
  openuiApply(openuiTableSet, wasm.__wbindgen_externrefs, [idx, obj]);
  return idx;
};
{
  let view = null, viewOf = null;
  getDataViewMemory0 = function () {
    const buffer = openuiApply(openuiMemoryBuffer, wasm.memory, []);
    if (view === null || viewOf !== buffer) { view = new openuiViewClass(buffer); viewOf = buffer; }
    const current = view;
    return { __proto__: null, setInt32: (offset, value, little) => openuiApply(openuiSetInt32, current, [offset, value, little]) };
  };
}

// The bundle reader passes a bounded UTF-16 copy to generated glue. Lower
// it using the captured typed-array intrinsics.
if (typeof passArray16ToWasm0 !== 'function') {
  throw new openuiError('openui package guard: generated UTF-16 helper is missing');
}
passArray16ToWasm0 = function (units, malloc) {
  const length = openuiApply(openuiUint16Length, units, []);
  const ptr = malloc(length * 2, 2) >>> 0;
  const buffer = openuiApply(openuiMemoryBuffer, wasm.memory, []);
  openuiApply(openuiUint16Set, new openuiUint16Array(buffer, ptr, length), [units]);
  WASM_VECTOR_LEN = length;
  return ptr;
};

// Argument conversions. None runs application code, except reading an
// action path's elements (an Array getter or Proxy), which happens while the
// guard is active, so it can neither reenter nor free anything.
// Host functions are the application-code call made from evaluation. Keep the
// guard active, including while extracting upstream's thrown-message text.
function openuiFunction(callback) {
  if (typeof callback !== 'function') throw openuiFailure('InvalidJson', 'expected a function callback');
  // Never throws (#208): an exception through Rust would end the instance.
  // A throw comes back as NUL, "threw", newline, then the message; a result
  // that itself starts with NUL comes back behind NUL, "value", newline, so
  // it cannot pass for one. JSON never starts with NUL (callbacks.rs).
  const wrapper = function (json) {
    let result;
    try { result = openuiApply(callback, undefined, [json]); }
    catch (error) {
      // Match callFunction in the pinned runtime/evaluator.ts:398-400. Error
      // message accessors and object conversion are host code too; reentry
      // from either is refused by the still-active package guard.
      let message;
      try { message = `${error instanceof openuiError ? error.message : openuiToString(error)}`; }
      catch { message = 'host function threw'; }
      return '\u0000threw\n' + message;
    }
    if (typeof result === 'string' && result.length > 0 && openuiApply(openuiCharCodeAt, result, [0]) === 0) {
      return '\u0000value\n' + result;
    }
    return result;
  };
  // The generated import calls `callback.call(undefined, text)`: an own
  // `call`, so a replaced Function.prototype.call never runs there (#217).
  openuiDefine(wrapper, 'call', { value: (thisArg, json) => wrapper(json), writable: false, enumerable: false, configurable: false });
  return wrapper;
}
const openuiString = value => {
  if (typeof value === 'string') return value;
  throw openuiFailure('InvalidJson', 'expected a string argument');
};
const openuiOptionalString = value => value === undefined || value === null ? undefined : openuiString(value);
const openuiBoolean = value => {
  if (typeof value === 'boolean') return value;
  throw openuiFailure('InvalidJson', 'expected a boolean argument');
};
// Read the response value under the package guard, before borrowing the session.
const openuiSessionLimits = new WeakMap();
const openuiEvaluationLimits = openuiDescriptor(RuntimeLimits.prototype, 'evaluation').get;
const openuiProgramSize = openuiDescriptor(ParseLimits.prototype, 'programSize').get;
const openuiFreeLimits = ParseLimits.prototype.free;
const openuiMaxContinuations = openuiDescriptor(RuntimeLimits.prototype, 'maxContinuations').get;
const openuiMaxJsonBytes = openuiDescriptor(RuntimeLimits.prototype, 'maxJsonBytes').get;
const openuiMaxActionPathBytes = openuiDescriptor(RuntimeLimits.prototype, 'maxActionPathBytes').get;
const openuiDefaultMaxJsonBytes = 8388608n;
const openuiIsSafeInteger = Number.isSafeInteger;
function openuiResponseState(value, receiver) {
  if (value === undefined) return undefined;
  if (value === null || typeof value !== 'object') throw openuiFailure('InvalidJson', 'expected a response state');
  const streaming = openuiBoolean(value.streaming);
  const bundle = value.isBundle, complete = value.complete;
  const error = openuiOptionalString(value.error), revision = openuiOptionalString(value.scriptRevision);
  const names = value.scripts;
  if (names !== undefined && !openuiIsArray(names)) throw openuiFailure('InvalidJson', 'expected a scripts array');
  const count = names === undefined ? 0 : names.length;
  if (!openuiIsSafeInteger(count) || count < 0) throw openuiFailure('InvalidJson', 'expected a scripts array length');
  // Bytes as Rust counts them: four bytes per name plus the UTF-8 of each
  // text as it crosses. A text's UTF-16 length, never more than its bytes, is
  // checked first, so no text longer than the limit is measured or copied.
  const limit = openuiGet(openuiSessionLimits, receiver)?.maxJsonBytes ?? openuiDefaultMaxJsonBytes;
  const refuse = () => { throw openuiFailure('LimitExceeded', 'response state byte limit'); };
  let used = count * 4 + (error?.length ?? 0) + (revision?.length ?? 0);
  if (used > limit) refuse();
  const scriptRevision = revision === undefined ? undefined : openuiMessageText(revision);
  used = count * 4 + (error === undefined ? 0 : openuiUtf8Bytes(error))
    + (scriptRevision === undefined ? 0 : openuiUtf8Bytes(scriptRevision));
  if (used > limit) refuse();
  // The scripts JSON handed to Rust is also held to the limit, in bytes:
  // escapes make it up to six times its names.
  let scripts = '[', json = 2;
  for (let i = 0; i < count; i++) {
    const name = openuiString(names[i]);
    if (used + name.length > limit) refuse();
    const text = openuiMessageText(name), quoted = openuiStringify(text);
    used += openuiUtf8Bytes(text);
    json += (i ? 1 : 0) + openuiUtf8Bytes(quoted);
    if (used > limit || json > limit) refuse();
    scripts += (i ? ',' : '') + quoted;
  }
  return { __proto__: null, streaming, isBundle: bundle === undefined ? false : openuiBoolean(bundle),
    complete: complete === undefined ? true : openuiBoolean(complete), scriptsJson: scripts + ']',
    error, scriptRevision };
}
// The UTF-8 bytes of `text` as wasm-bindgen sends it, a lone surrogate
// replaced by U+FFFD (three bytes). The count can stop once it exceeds budget.
function openuiUtf8Bytes(text, budget = Infinity) {
  let bytes = 0;
  for (let i = 0; i < text.length; i++) {
    const unit = openuiApply(openuiCharCodeAt, text, [i]);
    if (unit < 0x80) bytes += 1;
    else if (unit < 0x800) bytes += 2;
    else if (unit >= 0xd800 && unit <= 0xdbff && i + 1 < text.length) {
      const next = openuiApply(openuiCharCodeAt, text, [i + 1]);
      if (next >= 0xdc00 && next <= 0xdfff) { bytes += 4; i++; } else bytes += 3;
    } else bytes += 3;
    if (bytes > budget) break;
  }
  return bytes;
}
function openuiAdmitDefinition(args) {
  const text = args[0];
  const limit = args.length === 1 ? openuiDefaultMaxJsonBytes : openuiApply(openuiMaxJsonBytes, args[1], []);
  // As for scripts JSON, use the UTF-16 length as a fast lower bound and
  // count the UTF-8 bytes without allocating an encoded copy.
  if (text.length > limit || openuiUtf8Bytes(text) > limit) {
    throw openuiFailure('LimitExceeded', 'host input byte limit');
  }
}
const openuiLimit = value => {
  if (typeof value === 'bigint' && value >= 0n && value <= 18446744073709551615n) return value;
  throw openuiFailure('LimitExceeded', 'expected an unsigned 64-bit bigint limit');
};
const openuiOptionalLimit = value => value === undefined || value === null ? undefined : openuiLimit(value);
function openuiCapability(value, prototype) {
  if (!openuiHas(openuiMinted, value) || openuiGet(openuiClassOf, value) !== prototype) {
    throw openuiFailure('InvalidHandle', 'expected an opaque capability');
  }
  if (openuiHas(openuiFreed, value)) throw openuiFailure('Disposed', 'argument object freed');
  return value;
}
const openuiCapabilityOf = klass => {
  const prototype = klass.prototype;
  return value => openuiCapability(value, prototype);
};
// wasm-bindgen converts each UTF-16 string lossily; do the same per segment,
// so concatenation cannot pair surrogates across segments.
function openuiWellFormed(text) {
  if (typeof openuiToWellFormed === 'function') return openuiApply(openuiToWellFormed, text, []);
  let out = '';
  for (let i = 0; i < text.length; i++) {
    const unit = openuiApply(openuiCharCodeAt, text, [i]);
    if (unit >= 0xd800 && unit <= 0xdbff && i + 1 < text.length) {
      const next = openuiApply(openuiCharCodeAt, text, [i + 1]);
      if (next >= 0xdc00 && next <= 0xdfff) { out += openuiFromCharCode(unit, next); i++; continue; }
    }
    out += unit >= 0xd800 && unit <= 0xdfff ? '�' : openuiFromCharCode(unit);
  }
  return out;
}
// Message text can end in the middle of a surrogate pair. Carry lone units
// through wasm-bindgen's UTF-8 string transport using the core's stand-ins.
function openuiMessageText(value) {
  const text = openuiString(value);
  if (typeof openuiIsWellFormed === 'function' && openuiApply(openuiIsWellFormed, text, [])) return text;
  let out = '', start = 0;
  for (let i = 0; i < text.length; i++) {
    const unit = openuiApply(openuiCharCodeAt, text, [i]);
    if (unit < 0xd800 || unit > 0xdfff) continue;
    if (unit <= 0xdbff && i + 1 < text.length) {
      const next = openuiApply(openuiCharCodeAt, text, [i + 1]);
      if (next >= 0xdc00 && next <= 0xdfff) { i++; continue; }
    }
    const offset = unit - 0xd800;
    out += openuiApply(openuiSlice, text, [start, i]) + openuiFromCharCode(0xdbfe + (offset >> 10), 0xdc00 + (offset & 0x3ff));
    start = i + 1;
  }
  return start === 0 ? text : out + openuiApply(openuiSlice, text, [start]);
}
// Preserve UTF-16 units with no reserved scalars. Bound the temporary copy
// using the minimum possible number of UTF-8 bytes before allocating it.
function openuiResponseText(value, receiver) {
  return openuiResponseUnits(openuiResponseString(value, receiver), 0);
}
// A raw response admitted against the session's program-size limit.
function openuiResponseString(value, receiver) {
  const text = openuiOptionalString(value) ?? '';
  const length = text.length;
  if (length > 1024 * 1024 || length > (openuiGet(openuiSessionLimits, receiver)?.programSize ?? 1024 * 1024)) {
    throw openuiFailure('LimitExceeded', 'response bundle exceeds the program-size limit');
  }
  return text;
}
// The UTF-16 units of `text` from `from` on, lone surrogates included.
function openuiResponseUnits(text, from) {
  const length = text.length - from;
  const units = new openuiUint16Array(length);
  for (let i = 0; i < length; i++) units[i] = openuiApply(openuiCharCodeAt, text, [from + i]);
  return units;
}
// A copy of the caller's path as JSON text, so Rust reads no JavaScript
// values while it holds the session. A nonempty path with charge B uses at
// most 6B + 1 JSON bytes.
function openuiPath(value, receiver) {
  if (!openuiIsArray(value)) throw openuiFailure('InvalidHandle', 'expected a path array');
  const length = value.length;
  if (typeof length !== 'number') throw openuiFailure('InvalidHandle', 'expected a path array');
  const limits = openuiGet(openuiSessionLimits, receiver);
  const maxJsonBytes = limits?.maxJsonBytes ?? openuiDefaultMaxJsonBytes;
  const maxActionPathBytes = limits?.maxActionPathBytes;
  // A JS string cannot hold enough path text to reach the safe integer bound.
  let remaining = openuiNumber(maxActionPathBytes !== undefined && maxActionPathBytes < maxJsonBytes
    ? maxActionPathBytes : maxJsonBytes);
  let text = '[';
  for (let i = 0; i < length; i++) {
    const part = value[i];
    if (typeof part !== 'string') throw openuiFailure('InvalidHandle', 'expected string path segments');
    // Reserve one byte per segment. Its UTF-16 length is a lower bound on
    // its UTF-8 length. Empty text signals overflow to Rust, where session
    // and evaluation checks still run before the limit error.
    remaining -= 1;
    if (part.length > remaining) return '';
    remaining -= openuiUtf8Bytes(part, remaining);
    if (remaining < 0) return '';
    text += (i === 0 ? '' : ',') + openuiStringify(openuiWellFormed(part));
  }
  return text + ']';
}
// run() borrows the caller's action through a fresh token, taken after every
// other check, because wasm-bindgen cannot lower Option<&ActionHandle>.
const openuiActionPrototype = ActionHandle.prototype;
const openuiOptionalAction = value => value === undefined || value === null
  ? undefined : openuiCapability(value, openuiActionPrototype);
const openuiRetain = ActionHandle.prototype.retain;

// The public surface, member by member: one conversion per parameter, then
// an optional check of the converted arguments. `json` parses a JSON text
// result. `free` marks the receiver freed.
function openuiSpec(args, options = {}) {
  return openuiFreeze({ __proto__: null, args: openuiFreeze(args),
    check: openuiHasOwn(options, 'check') ? options.check : undefined, json: options.json === true,
    free: options.free === true, retainFirst: options.retainFirst === true });
}
const openuiFree = openuiSpec([], { free: true });
const openuiNone = openuiSpec([]);
const openuiJson = openuiSpec([], { json: true });
const openuiStr = openuiString, openuiOptStr = openuiOptionalString, openuiBool = openuiBoolean;
const openuiLimitField = { get: openuiNone, set: openuiSpec([openuiLimit]) };
const openuiOptionalLimitField = { get: openuiNone, set: openuiSpec([openuiOptionalLimit]) };
const openuiParseLimitsField = { get: openuiNone, set: openuiSpec([openuiCapabilityOf(ParseLimits)]) };
const openuiLibrary = openuiCapabilityOf(ComponentLibrary);
const openuiSurface = new Map([
  [RuntimeSession, {
    construct: openuiSpec([openuiLibrary, openuiOptStr]),
    statics: {
      withLimits: openuiSpec([openuiLibrary, openuiOptStr, openuiCapabilityOf(RuntimeLimits)]),
    },
    methods: {
      free: openuiFree,
      actionAt: openuiSpec([openuiCapabilityOf(EvaluationHandle), openuiPath]),
      dispose: openuiNone,
      evaluate: openuiSpec([(value, session) => value === undefined
        ? openuiGet(openuiRawDraws, session)?.state : openuiResponseState(value, session)]),
      attachToolProvider: openuiNone,
      detachToolProvider: openuiNone,
      setQueryExtraValues: openuiSpec([openuiStr, openuiStr]),
      takeQueryUpdate: openuiNone,
      retryFailedQueries: openuiNone,
      sampleQueryClock: openuiSpec([openuiLimit]),
      advanceQueryClock: openuiSpec([openuiLimit]),
      pauseQueries: openuiNone,
      resumeQueries: openuiNone,
      resumeQueriesAt: openuiSpec([openuiLimit]),
      attachToolProviderAt: openuiSpec([openuiLimit, openuiLimit]),
      attachToolProviderWithMinimum: openuiSpec([openuiLimit]),
      completeToolCall: openuiSpec([openuiCapabilityOf(WantedToolCall), openuiStr, value => value == null ? false : openuiBool(value)]),
      get: openuiSpec([openuiStr]),
      getField: openuiSpec([openuiOptStr, openuiStr]),
      initialize: openuiSpec([openuiOptStr], { json: true }),
      initializeField: openuiSpec([openuiOptStr, openuiOptStr, openuiStr, openuiOptStr, openuiBool], { json: true }),
      isPristine: openuiSpec([openuiStr]),
      markBinding: openuiSpec([openuiStr, openuiStr]),
      pendingContinuations: openuiNone,
      registerFunction: openuiSpec([openuiStr, openuiFunction]),
      resume: openuiSpec([openuiCapabilityOf(ContinuationHandle), openuiBool]),
      run: openuiSpec([openuiOptionalAction, openuiStr, openuiOptStr], { retainFirst: true }),
      runLegacy: openuiSpec([openuiStr, openuiStr, openuiOptStr]),
      set: openuiSpec([openuiStr, openuiOptStr]),
      setField: openuiSpec([openuiOptStr, openuiOptStr, openuiStr, openuiOptStr, openuiBool], { json: true }),
      replaceProgram: openuiSpec([openuiCapabilityOf(ParsedProgram), openuiOptStr, openuiOptStr, openuiBool, openuiResponseState]),
      setProgram: openuiSpec([openuiCapabilityOf(ParsedProgram)]),
      setReferenceValues: openuiSpec([openuiStr]),
      snapshot: openuiNone,
      undecidedBindings: openuiJson,
      version: openuiNone,
      writeBinding: openuiSpec([openuiStr, openuiStr]),
    },
  }],
  [WantedToolCall, { methods: { free: openuiFree }, accessors: {
    pending: { get: openuiNone }, toolName: { get: openuiNone }, argumentsJson: { get: openuiNone } } }],
  [QueryEffects, { methods: { free: openuiFree, actionAt: openuiSpec([openuiLimit]), callAt: openuiSpec([openuiLimit]) }, accessors: {
    nextRefreshDueMs: { get: openuiNone }, readerRoundsLimited: { get: openuiNone }, actionCount: { get: openuiNone }, callCount: { get: openuiNone }, changed: { get: openuiNone }, evaluationError: { get: openuiJson } } }],
  [EvaluationHandle, { methods: { free: openuiFree } }],
  [ActionHandle, { methods: { free: openuiFree, retain: openuiNone } }],
  [ContinuationHandle, { methods: { free: openuiFree } }],
  [ParsedProgram, { methods: { free: openuiFree } }],
  [EvaluationOutput, {
    methods: { free: openuiFree },
    accessors: { blocked: { get: openuiNone }, isQueryLoading: { get: openuiNone }, parserErrorJson: { get: openuiNone }, queryEvaluationError: { get: openuiJson }, querySnapshotJson: { get: openuiNone }, evaluation: { get: openuiNone }, json: { get: openuiNone },
      sizeLimited: { get: openuiNone }, depthLimited: { get: openuiNone } },
  }],
  [ReplacementOutput, {
    methods: { free: openuiFree },
    accessors: { initialization: { get: openuiJson }, projection: { get: openuiJson },
      evaluation: { get: openuiNone }, snapshot: { get: openuiNone }, version: { get: openuiNone } },
  }],
  [ActionOutput, {
    methods: { free: openuiFree },
    accessors: { continuation: { get: openuiNone }, events: { get: openuiJson },
      failure: { get: openuiJson }, sizeLimited: { get: openuiNone }, depthLimited: { get: openuiNone } },
  }],
  [RuntimeLimits, {
    construct: openuiNone,
    methods: { free: openuiFree },
    accessors: { evaluation: openuiParseLimitsField, referencePreparation: openuiParseLimitsField,
      maxJsonBytes: openuiLimitField, maxActionPathBytes: openuiOptionalLimitField, maxContinuations: openuiLimitField,
      maxCacheBytes: openuiOptionalLimitField },
  }],
  [ParseLimits, {
    construct: openuiNone,
    methods: { free: openuiFree },
    accessors: { programSize: openuiOptionalLimitField, resultSize: openuiOptionalLimitField,
      resultFloor: openuiOptionalLimitField, amplification: openuiOptionalLimitField,
      resultCeiling: openuiOptionalLimitField, depth: openuiOptionalLimitField,
      work: openuiOptionalLimitField, peakMemory: openuiOptionalLimitField },
  }],
  // A library's JSON Schema and a library definition are host input, held
  // to maxJsonBytes in bytes by Rust before they are read. The guard also
  // checks a definition before wasm-bindgen copies it. A definition's JSON
  // Schema and prompt spec come back as text. A defined library's
  // prompt and cloud config take their options and instructions as text,
  // held by Rust to the limits the library was made with (#135).
  [ComponentLibrary, {
    construct: openuiSpec([openuiStr]),
    statics: {
      fromSchemaWithLimits: openuiSpec([openuiStr, openuiCapabilityOf(RuntimeLimits)]),
      fromDefinition: openuiSpec([openuiStr], { check: openuiAdmitDefinition }),
      fromDefinitionWithLimits: openuiSpec([openuiStr, openuiCapabilityOf(RuntimeLimits)], { check: openuiAdmitDefinition }),
    },
    methods: { free: openuiFree, jsonSchema: openuiNone, promptSpec: openuiNone,
      prompt: openuiSpec([openuiOptStr]), cloudConfig: openuiSpec([openuiOptStr, openuiOptStr]) },
  }],
  [StreamSession, {
    construct: openuiSpec([openuiLibrary, openuiOptStr]),
    methods: { free: openuiFree, feed: openuiSpec([openuiStr]), finish: openuiNone,
      program: openuiSpec([openuiBool]), push: openuiSpec([openuiStr]), result: openuiNone,
      set: openuiSpec([openuiStr]) },
  }],
]);
// Each class's finalization registry, for the rebuilt __wrap. A class the
// table lacks stops the module from loading, as one the surface lacks does.
const openuiRegistries = new Map([
  [RuntimeSession, RuntimeSessionFinalization], [WantedToolCall, WantedToolCallFinalization],
  [QueryEffects, QueryEffectsFinalization], [EvaluationHandle, EvaluationHandleFinalization],
  [ActionHandle, ActionHandleFinalization], [ContinuationHandle, ContinuationHandleFinalization],
  [ParsedProgram, ParsedProgramFinalization], [EvaluationOutput, EvaluationOutputFinalization],
  [ReplacementOutput, ReplacementOutputFinalization], [ActionOutput, ActionOutputFinalization],
  [RuntimeLimits, RuntimeLimitsFinalization], [ParseLimits, ParseLimitsFinalization],
  [ComponentLibrary, ComponentLibraryFinalization], [StreamSession, StreamSessionFinalization],
]);
for (const klass of openuiSurface.keys()) {
  if (!openuiRegistries.has(klass)) throw new openuiError(`openui package guard: ${klass.name} has no finalization registry`);
}
const openuiPrototypes = new WeakSet();
for (const klass of openuiSurface.keys()) openuiAdd(openuiPrototypes, klass.prototype);

// Pointer fields become accessors: readable, writable only by generated code
// inside a guarded call and only to 0 (its free). They cannot be forged.
function openuiPointer() {
  const pointer = openuiGet(openuiPointers, this);
  return pointer === undefined ? 0 : pointer;
}
function openuiSetPointer(value) {
  if (!openuiActive || value !== 0) throw openuiFailure('InvalidHandle', 'pointer fields are private');
  openuiPut(openuiPointers, this, 0);
}
// Brand an object the package returned, so it can be a receiver or capability.
function openuiRemember(value) {
  if (value === null || typeof value !== 'object') return value;
  const prototype = openuiPrototypeOf(value);
  if (!openuiHas(openuiPrototypes, prototype) || openuiHas(openuiMinted, value)) return value;
  const field = openuiDescriptor(value, '__wbg_ptr');
  if (field === undefined || !openuiHasOwn(field, 'value')) return value;
  openuiPut(openuiPointers, value, field.value);
  openuiDefine(value, '__wbg_ptr', { get: openuiPointer, set: openuiSetPointer, enumerable: false, configurable: false });
  openuiPut(openuiClassOf, value, prototype);
  openuiAdd(openuiMinted, value);
  return value;
}

// An exception that leaves a wasm export was thrown through wasm frames, and
// skipped every Rust epilogue and destructor on the way out (#208): a
// JavaScript stack overflow inside wasm, a trap, or a host import throwing.
// Rust's own errors never do that (generated code throws them after Rust has
// returned), and the guard's refusals happen before Rust is entered. Every
// raw export is called through a wrapper that sees such an exception at
// once. From then on the instance calls no more Rust: every guarded call
// refuses, and every raw export is a no-op, so the generated glue's own
// clean-up after the escape, every free and wasm-bindgen's finalizers, which
// call the same free exports, enter nothing. The shadow stack pointer is put back all the
// same. The host must load the package again.
let openuiInterrupted = false;
let openuiIdleStack;
function openuiRefuseInterrupted() {
  if (openuiInterrupted) {
    throw openuiFailure('Disposed', 'the WebAssembly instance was interrupted by an exception it cannot recover from; load the package again');
  }
}
function openuiEscaped(raw) {
  try { openuiApply(openuiGlobalValue.set, raw.__stack_pointer, [openuiIdleStack]); }
  catch { /* no room left: nothing runs on that stack again */ }
}
// The generated module calls every export through its `wasm` binding;
// replace that binding with wrappers (rule 1 does not apply: it is private).
function openuiWrapExports(raw) {
  openuiIdleStack = openuiApply(openuiGlobalValue.get, raw.__stack_pointer, []);
  const wrapped = { __proto__: null };
  for (const name of openuiOwnKeys(raw)) {
    const value = raw[name];
    if (typeof value !== 'function') { wrapped[name] = value; continue; }
    // Once refusing, no export runs, not even the frees and allocations the
    // generated glue makes in catch and finally blocks of the call that
    // escaped. Allocators answer 0 (a null pointer); everything else nothing.
    const pointer = name === '__wbindgen_malloc' || name === '__wbindgen_realloc' || name === '__externref_table_alloc';
    const refused = pointer ? 0 : undefined;
    wrapped[name] = function () {
      if (openuiInterrupted) return refused;
      try { return openuiApply(value, raw, arguments); }
      catch (error) {
        // First, before any call can fail: the instance refuses from here.
        openuiInterrupted = true;
        openuiEscaped(raw);
        throw error;
      }
    };
  }
  return openuiFreeze(wrapped);
}
if (typeof __wbg_finalize_init === 'function') {
  // The web target instantiates later, in init(); wrap when it does.
  const openuiFinalizeInit = __wbg_finalize_init;
  __wbg_finalize_init = function (instance, module) {
    const exports = openuiApply(openuiFinalizeInit, undefined, [instance, module]);
    wasm = openuiWrapExports(exports);
    return exports;
  };
} else {
  wasm = openuiWrapExports(wasm);
}

function openuiConvert(conversions, args, receiver) {
  const converted = [];
  for (let i = 0; i < conversions.length; i++) {
    const value = conversions[i](i < args.length ? args[i] : undefined, receiver);
    openuiDefine(converted, i, { value, writable: true, enumerable: true, configurable: true });
  }
  return converted;
}
const openuiSessionPrototype = RuntimeSession.prototype;

function openuiRefuseReentry() {
  if (openuiActive) throw openuiFailure('Worker', 'session already in use');
}

// One guarded call: refuse reentry, convert, check the receiver, call, then
// build any JSON record. `prototype` is undefined for constructors, statics
// and free functions, which have no receiver.
function openuiCall(spec, receiver, prototype, args, call) {
  openuiRefuseReentry();
  if (prototype !== undefined) {
    if (!openuiHas(openuiMinted, receiver) || openuiGet(openuiClassOf, receiver) !== prototype) {
      throw openuiFailure('InvalidHandle', 'invalid object receiver');
    }
    if (openuiHas(openuiFreed, receiver)) {
      if (spec.free) return undefined;
      throw openuiFailure('Disposed', 'object freed');
    }
  }
  let interrupted = false;
  if (openuiInterrupted && spec.free) {
    // No guarded call enters an instance that cannot be trusted, not even a
    // destructor. Release the JavaScript object (which also cancels its
    // finalizer) and leave its Rust value to go with the instance. The
    // helper comes from the frozen class prototype, never the receiver.
    // (wasm-bindgen's finalizers for objects never freed still call in.)
    openuiActive = true;
    try { openuiApply(openuiDescriptor(prototype, '__destroy_into_raw').value, receiver, []); }
    finally {
      openuiActive = false;
      openuiAdd(openuiFreed, receiver);
    }
    return undefined;
  }
  openuiRefuseInterrupted();
  openuiActive = true;
  try {
    const converted = openuiConvert(spec.args, args, receiver);
    // Conversion cannot free anything (every free refuses while active), so
    // this still holds when the generated code reads the pointers.
    if (prototype !== undefined && openuiHas(openuiFreed, receiver)) throw openuiFailure('Disposed', 'object freed');
    if (spec.check !== undefined) spec.check(converted);
    if (spec.retainFirst && converted[0] !== undefined) converted[0] = openuiApply(openuiRetain, converted[0], []);
    let result;
    try { result = call(converted); }
    catch (error) {
      // An escape: the instance already refuses. Stop this session's driver
      // (JavaScript only) and call nothing more.
      if (openuiInterrupted && prototype === openuiSessionPrototype) interrupted = true;
      throw error;
    } finally { if (spec.free) openuiAdd(openuiFreed, receiver); }
    if (spec.json) result = openuiParseJson(result);
    return openuiRemember(result);
  } catch (error) {
    throw openuiTyped(error);
  } finally {
    openuiActive = false;
    if (interrupted) openuiEndSession(receiver);
  }
}
function openuiMember(spec, prototype, original) {
  return function (...args) {
    return openuiCall(spec, this, prototype, args, converted => openuiApply(original, this, converted));
  };
}
// Private generated helpers (__wrap, __destroy_into_raw) are called only by
// generated code, which always runs inside a guarded call.
function openuiPrivate(original) {
  return function (...args) {
    if (!openuiActive) throw openuiFailure('InvalidHandle', 'private generated helper');
    return openuiApply(original, this, args);
  };
}
const openuiUncovered = (name, key) => new openuiError(
  `openui package guard: ${name}.${String(key)} is not covered; update package-guards.js`);
// Generated free calls this.__destroy_into_raw() through the instance, which
// a host can redirect to another object's pointer: an own property, a
// subclass field, a prototype set with setPrototypeOf (#211). Run it on a
// private stand-in whose one member is the helper captured at load, applied
// to the checked receiver. Every other generated method reaches only the
// pointer field through an instance, an own accessor nothing can shadow
// (pointer-redirection.test.ts scans the generated code for this).
function openuiReleasing(generatedFree, destroy) {
  return function () {
    const receiver = this;
    const standIn = { __proto__: null, __destroy_into_raw: () => openuiApply(destroy, receiver, []) };
    return openuiApply(generatedFree, standIn, []);
  };
}

function openuiGuardClass(klass, spec) {
  const prototype = klass.prototype, name = klass.name;
  const methods = spec.methods ?? {}, accessors = spec.accessors ?? {}, statics = spec.statics ?? {};
  const seen = { __proto__: null };
  let guardedFree;
  const destroyField = openuiDescriptor(prototype, '__destroy_into_raw');
  const generatedDestroy = destroyField !== undefined && openuiHasOwn(destroyField, 'value') ? destroyField.value : undefined;
  if (typeof generatedDestroy !== 'function') throw openuiUncovered(name, '__destroy_into_raw (missing)');
  const destroy = openuiDestroyer(openuiRegistries.get(klass));
  for (const key of openuiOwnKeys(prototype)) {
    if (key === 'constructor' || key === openuiDisposeSymbol) continue;
    const descriptor = openuiDescriptor(prototype, key);
    if (openuiIsPrivate(key)) {
      if (typeof descriptor.value !== 'function') throw openuiUncovered(name, key);
      descriptor.value = openuiPrivate(key === '__destroy_into_raw' ? destroy : descriptor.value);
    } else if (typeof key !== 'string') {
      throw openuiUncovered(name, key);
    } else if (openuiHasOwn(descriptor, 'value')) {
      const member = openuiHasOwn(methods, key) ? methods[key] : undefined;
      if (member === undefined || typeof descriptor.value !== 'function') throw openuiUncovered(name, key);
      const generated = key === 'free' ? openuiReleasing(descriptor.value, destroy) : descriptor.value;
      descriptor.value = klass === RuntimeSession
        ? openuiDriverMember(key, member, prototype, generated)
        : openuiMember(member, prototype, generated);
      if (key === 'free') guardedFree = descriptor.value;
      seen[key] = true;
    } else {
      const member = openuiHasOwn(accessors, key) ? accessors[key] : undefined;
      if (member === undefined) throw openuiUncovered(name, key);
      for (const slot of ['get', 'set']) {
        if (descriptor[slot] === undefined && member[slot] === undefined) continue;
        if (member[slot] === undefined) throw openuiUncovered(name, `${key} (${slot})`);
        if (descriptor[slot] === undefined) throw openuiUncovered(name, `${key} (${slot} missing)`);
        descriptor[slot] = openuiMember(member[slot], prototype, descriptor[slot]);
      }
      seen[key] = true;
    }
    openuiDefine(prototype, key, descriptor);
  }
  for (const key of openuiOwnKeys(methods)) if (seen[key] !== true) throw openuiUncovered(name, `${key} (missing)`);
  for (const key of openuiOwnKeys(accessors)) if (seen[key] !== true) throw openuiUncovered(name, `${key} (missing)`);
  if (guardedFree === undefined) throw openuiUncovered(name, 'free (missing)');
  // wasm-bindgen sets [Symbol.dispose] = free at load: route it to the guard.
  if (openuiDisposeSymbol !== undefined) {
    openuiDefine(prototype, openuiDisposeSymbol, { value: guardedFree, writable: false, enumerable: false, configurable: false });
  }
  for (const key of openuiOwnKeys(klass)) {
    if (key === 'length' || key === 'name' || key === 'prototype') continue;
    const descriptor = openuiDescriptor(klass, key);
    if (openuiIsPrivate(key)) {
      if (typeof descriptor.value !== 'function') throw openuiUncovered(name, key);
      // __wrap is rebuilt on intrinsics captured at load (#217); any other
      // private static stays the generated one, usable only inside a call.
      descriptor.value = openuiPrivate(key === '__wrap' ? openuiWrapper(klass, openuiRegistries.get(klass)) : descriptor.value);
    } else {
      const member = typeof key === 'string' && openuiHasOwn(statics, key) ? statics[key] : undefined;
      if (member === undefined || typeof descriptor.value !== 'function') throw openuiUncovered(name, key);
      const original = descriptor.value;
      descriptor.value = function (...args) {
        return openuiCall(member, undefined, undefined, args, converted => {
          const result = openuiApply(original, klass, converted);
          if (key === 'withLimits' && name === 'RuntimeSession') {
            const evaluation = openuiApply(openuiEvaluationLimits, converted[2], []);
            try {
              openuiPut(openuiSessionLimits, result, {__proto__: null,
                maxContinuations: openuiApply(openuiMaxContinuations, converted[2], []),
                maxJsonBytes: openuiApply(openuiMaxJsonBytes, converted[2], []),
                maxActionPathBytes: openuiApply(openuiMaxActionPathBytes, converted[2], []),
                programSize: openuiApply(openuiProgramSize, evaluation, [])});
            } finally { openuiApply(openuiFreeLimits, evaluation, []); }
          }
          return result;
        });
      };
    }
    openuiDefine(klass, key, descriptor);
  }
  for (const key of openuiOwnKeys(statics)) if (!openuiHasOwn(klass, key)) throw openuiUncovered(name, `${key} (missing)`);
  // Nothing can replace a guarded member, or a private helper that the
  // guarded free calls, after this.
  if (klass === RuntimeSession) openuiInstallDriver(prototype);
  const construct = spec.construct;
  const guarded = new openuiProxy(klass, {
    construct(target, args) {
      if (construct === undefined) throw openuiFailure('InvalidHandle', 'opaque object has no constructor');
      return openuiCall(construct, undefined, undefined, args, converted => openuiConstruct(target, converted, target));
    },
  });
  // instance.constructor is the guarded class too: the generated constructor
  // reads its arguments' pointers unchecked, so it must not be reachable (#211).
  openuiDefine(prototype, 'constructor', { value: guarded, writable: false, enumerable: false, configurable: false });
  openuiFreeze(prototype);
  openuiFreeze(klass);
  return guarded;
}

const openuiGuarded = new Map();
for (const [klass, spec] of openuiSurface) openuiGuarded.set(klass, openuiGuardClass(klass, spec));
RuntimeSession = openuiGuarded.get(RuntimeSession);
EvaluationHandle = openuiGuarded.get(EvaluationHandle);
ActionHandle = openuiGuarded.get(ActionHandle);
ContinuationHandle = openuiGuarded.get(ContinuationHandle);
ParsedProgram = openuiGuarded.get(ParsedProgram);
EvaluationOutput = openuiGuarded.get(EvaluationOutput);
ActionOutput = openuiGuarded.get(ActionOutput);
ReplacementOutput = openuiGuarded.get(ReplacementOutput);
RuntimeLimits = openuiGuarded.get(RuntimeLimits);
ParseLimits = openuiGuarded.get(ParseLimits);
ComponentLibrary = openuiGuarded.get(ComponentLibrary);
StreamSession = openuiGuarded.get(StreamSession);

const openuiParseText = parse, openuiParseProgram = parseProgram;
const openuiParseSpec = openuiSpec([openuiStr, openuiLibrary, openuiOptStr]);
parse = (...args) =>
  openuiCall(openuiParseSpec, undefined, undefined, args, converted => openuiApply(openuiParseText, undefined, converted));
parseProgram = (...args) =>
  openuiCall(openuiParseSpec, undefined, undefined, args, converted => openuiApply(openuiParseProgram, undefined, converted));

const openuiParseMessage = parseMessage, openuiBuildMessage = buildMessage;
const openuiParseMessageSpec = openuiSpec([openuiMessageText, openuiBool]);
const openuiBuildMessageSpec = openuiSpec([openuiMessageText], { json: true });
parseMessage = (...args) =>
  openuiCall(openuiParseMessageSpec, undefined, undefined, args, converted => openuiApply(openuiParseMessage, undefined, converted));
buildMessage = (...args) =>
  openuiCall(openuiBuildMessageSpec, undefined, undefined, args, converted => openuiApply(openuiBuildMessage, undefined, converted));

const openuiParseResponseBundle = parseResponseBundle;
const openuiResponseBundleSpec = openuiSpec([openuiResponseText, openuiBool]);
parseResponseBundle = (...args) =>
  openuiCall(openuiResponseBundleSpec, undefined, undefined, args, converted => openuiApply(openuiParseResponseBundle, undefined, converted));

// A free function like the readers above, as in the native binding.
const openuiExtractMcpToolResult = extractMcpToolResult;
const openuiExtractMcpSpec = openuiSpec([openuiStr, openuiCapabilityOf(ParseLimits)]);
extractMcpToolResult = (...args) =>
  openuiCall(openuiExtractMcpSpec, undefined, undefined, args, converted => openuiApply(openuiExtractMcpToolResult, undefined, converted));

// CommonJS exports captured the unguarded values; web exports are live
// bindings. Refuse to load if the package exports anything not guarded here.
if (typeof exports !== 'undefined') {
  const openuiExports = { RuntimeSession, EvaluationHandle, ActionHandle, ContinuationHandle,
    ParsedProgram, EvaluationOutput, ActionOutput, ReplacementOutput, RuntimeLimits, ParseLimits, ComponentLibrary,
    StreamSession, parse, parseProgram, parseMessage, buildMessage, parseResponseBundle, extractMcpToolResult };
  for (const key of openuiOwnKeys(exports)) {
    if (key !== '__esModule' && !openuiHasOwn(openuiExports, key)) throw openuiUncovered('exports', key);
  }
  Object.assign(exports, openuiExports);
}
