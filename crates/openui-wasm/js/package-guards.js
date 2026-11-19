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
const openuiDescriptor = Object.getOwnPropertyDescriptor;
const openuiPrototypeOf = Object.getPrototypeOf;
const openuiHasOwn = Object.hasOwn;
const openuiFreeze = Object.freeze;
const openuiIsArray = Array.isArray;
const openuiParseJson = JSON.parse;
const openuiStringify = JSON.stringify;
const openuiError = Error;
const openuiToString = String;
const openuiProxy = Proxy;
const openuiSetHas = WeakSet.prototype.has;
const openuiSetAdd = WeakSet.prototype.add;
const openuiMapGet = WeakMap.prototype.get;
const openuiMapSet = WeakMap.prototype.set;
const openuiSlice = String.prototype.slice;
const openuiIndexOf = String.prototype.indexOf;
const openuiCharCodeAt = String.prototype.charCodeAt;
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
  LimitExceeded: true, Worker: true, InvalidSchema: true, InvalidPromptSpec: true, Prompt: true };
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
  return function (json) {
    try { return openuiApply(callback, undefined, [json]); }
    catch (error) {
      // Match callFunction in the pinned runtime/evaluator.ts:398-400. Error
      // message accessors and object conversion are host code too; reentry
      // from either is refused by the still-active package guard.
      throw `${error instanceof openuiError ? error.message : openuiToString(error)}`;
    }
  };
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
function openuiResponseState(value) {
  if (value === undefined) return undefined;
  if (value === null || typeof value !== 'object') throw openuiFailure('InvalidJson', 'expected a response state');
  return { __proto__: null, streaming: openuiBoolean(value.streaming) };
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
function openuiResponseText(value) {
  const text = openuiOptionalString(value) ?? '';
  const length = text.length;
  if (length > 1024 * 1024) {
    throw openuiFailure('LimitExceeded', 'response bundle exceeds the program-size limit');
  }
  const units = new openuiUint16Array(length);
  for (let i = 0; i < length; i++) units[i] = openuiApply(openuiCharCodeAt, text, [i]);
  return units;
}
// A copy of the caller's path as JSON text, so Rust reads no JavaScript
// values while it holds the session.
function openuiPath(value) {
  if (!openuiIsArray(value)) throw openuiFailure('InvalidHandle', 'expected a path array');
  const length = value.length;
  if (typeof length !== 'number') throw openuiFailure('InvalidHandle', 'expected a path array');
  let text = '[';
  for (let i = 0; i < length; i++) {
    const part = value[i];
    if (typeof part !== 'string') throw openuiFailure('InvalidHandle', 'expected string path segments');
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

// The public surface, member by member: one conversion per parameter; `json`
// parses a JSON text result; `free` marks the receiver freed.
function openuiSpec(args, options = {}) {
  return openuiFreeze({ __proto__: null, args: openuiFreeze(args), json: options.json === true,
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
      extractMcpToolResult: openuiSpec([openuiStr, openuiCapabilityOf(ParseLimits)]),
    },
    methods: {
      free: openuiFree,
      actionAt: openuiSpec([openuiCapabilityOf(EvaluationHandle), openuiPath]),
      dispose: openuiNone,
      evaluate: openuiSpec([openuiResponseState]),
      attachToolProvider: openuiNone,
      detachToolProvider: openuiNone,
      takeQueryUpdate: openuiNone,
      completeToolCall: openuiSpec([openuiCapabilityOf(WantedToolCall), openuiStr]),
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
    toolName: { get: openuiNone }, argumentsJson: { get: openuiNone } } }],
  [QueryEffects, { methods: { free: openuiFree, actionAt: openuiSpec([openuiLimit]), callAt: openuiSpec([openuiLimit]) }, accessors: {
    actionCount: { get: openuiNone }, callCount: { get: openuiNone }, changed: { get: openuiNone }, evaluationError: { get: openuiJson } } }],
  [EvaluationHandle, { methods: { free: openuiFree } }],
  [ActionHandle, { methods: { free: openuiFree, retain: openuiNone } }],
  [ContinuationHandle, { methods: { free: openuiFree } }],
  [ParsedProgram, { methods: { free: openuiFree } }],
  [EvaluationOutput, {
    methods: { free: openuiFree },
    accessors: { queryEvaluationError: { get: openuiJson }, querySnapshotJson: { get: openuiNone }, evaluation: { get: openuiNone }, json: { get: openuiNone },
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
      maxJsonBytes: openuiLimitField, maxActionPathBytes: openuiOptionalLimitField, maxContinuations: openuiLimitField },
  }],
  [ParseLimits, {
    construct: openuiNone,
    methods: { free: openuiFree },
    accessors: { programSize: openuiOptionalLimitField, resultSize: openuiOptionalLimitField,
      resultFloor: openuiOptionalLimitField, amplification: openuiOptionalLimitField,
      resultCeiling: openuiOptionalLimitField, depth: openuiOptionalLimitField,
      work: openuiOptionalLimitField, peakMemory: openuiOptionalLimitField },
  }],
  [ComponentLibrary, { construct: openuiSpec([openuiStr]), methods: { free: openuiFree } }],
  [StreamSession, {
    construct: openuiSpec([openuiLibrary, openuiOptStr]),
    methods: { free: openuiFree, feed: openuiSpec([openuiStr]), finish: openuiNone,
      program: openuiSpec([openuiBool]), push: openuiSpec([openuiStr]), result: openuiNone,
      set: openuiSpec([openuiStr]) },
  }],
]);
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

function openuiConvert(conversions, args) {
  const converted = [];
  for (let i = 0; i < conversions.length; i++) {
    const value = conversions[i](i < args.length ? args[i] : undefined);
    openuiDefine(converted, i, { value, writable: true, enumerable: true, configurable: true });
  }
  return converted;
}
// One guarded call: refuse reentry, convert, check the receiver, call, then
// build any JSON record. `prototype` is undefined for constructors, statics
// and free functions, which have no receiver.
function openuiCall(spec, receiver, prototype, args, call) {
  if (openuiActive) throw openuiFailure('Worker', 'session already in use');
  if (prototype !== undefined) {
    if (!openuiHas(openuiMinted, receiver) || openuiGet(openuiClassOf, receiver) !== prototype) {
      throw openuiFailure('InvalidHandle', 'invalid object receiver');
    }
    if (openuiHas(openuiFreed, receiver)) {
      if (spec.free) return undefined;
      throw openuiFailure('Disposed', 'object freed');
    }
  }
  openuiActive = true;
  try {
    const converted = openuiConvert(spec.args, args);
    // Conversion cannot free anything (every free refuses while active), so
    // this still holds when the generated code reads the pointers.
    if (prototype !== undefined && openuiHas(openuiFreed, receiver)) throw openuiFailure('Disposed', 'object freed');
    if (spec.retainFirst && converted[0] !== undefined) converted[0] = openuiApply(openuiRetain, converted[0], []);
    let result;
    try { result = call(converted); }
    finally { if (spec.free) openuiAdd(openuiFreed, receiver); }
    if (spec.json) result = openuiParseJson(result);
    return openuiRemember(result);
  } catch (error) {
    throw openuiTyped(error);
  } finally {
    openuiActive = false;
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

function openuiGuardClass(klass, spec) {
  const prototype = klass.prototype, name = klass.name;
  const methods = spec.methods ?? {}, accessors = spec.accessors ?? {}, statics = spec.statics ?? {};
  const seen = { __proto__: null };
  let guardedFree;
  for (const key of openuiOwnKeys(prototype)) {
    if (key === 'constructor' || key === openuiDisposeSymbol) continue;
    const descriptor = openuiDescriptor(prototype, key);
    if (openuiIsPrivate(key)) {
      if (typeof descriptor.value !== 'function') throw openuiUncovered(name, key);
      descriptor.value = openuiPrivate(descriptor.value);
    } else if (typeof key !== 'string') {
      throw openuiUncovered(name, key);
    } else if (openuiHasOwn(descriptor, 'value')) {
      const member = openuiHasOwn(methods, key) ? methods[key] : undefined;
      if (member === undefined || typeof descriptor.value !== 'function') throw openuiUncovered(name, key);
      descriptor.value = openuiMember(member, prototype, descriptor.value);
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
      descriptor.value = openuiPrivate(descriptor.value);
    } else {
      const member = typeof key === 'string' && openuiHasOwn(statics, key) ? statics[key] : undefined;
      if (member === undefined || typeof descriptor.value !== 'function') throw openuiUncovered(name, key);
      const original = descriptor.value;
      descriptor.value = function (...args) {
        return openuiCall(member, undefined, undefined, args, converted => openuiApply(original, klass, converted));
      };
    }
    openuiDefine(klass, key, descriptor);
  }
  for (const key of openuiOwnKeys(statics)) if (!openuiHasOwn(klass, key)) throw openuiUncovered(name, `${key} (missing)`);
  // Nothing can replace a guarded member, or a private helper that the
  // guarded free calls, after this.
  openuiFreeze(prototype);
  openuiFreeze(klass);
  const construct = spec.construct;
  return new openuiProxy(klass, {
    construct(target, args) {
      if (construct === undefined) throw openuiFailure('InvalidHandle', 'opaque object has no constructor');
      return openuiCall(construct, undefined, undefined, args, converted => openuiConstruct(target, converted, target));
    },
  });
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
parse = function (...args) {
  return openuiCall(openuiParseSpec, undefined, undefined, args, converted => openuiApply(openuiParseText, undefined, converted));
};
parseProgram = function (...args) {
  return openuiCall(openuiParseSpec, undefined, undefined, args, converted => openuiApply(openuiParseProgram, undefined, converted));
};

const openuiParseMessage = parseMessage, openuiBuildMessage = buildMessage;
const openuiParseMessageSpec = openuiSpec([openuiMessageText, openuiBool]);
const openuiBuildMessageSpec = openuiSpec([openuiMessageText], { json: true });
parseMessage = function (...args) {
  return openuiCall(openuiParseMessageSpec, undefined, undefined, args, converted => openuiApply(openuiParseMessage, undefined, converted));
};
buildMessage = function (...args) {
  return openuiCall(openuiBuildMessageSpec, undefined, undefined, args, converted => openuiApply(openuiBuildMessage, undefined, converted));
};

const openuiParseResponseBundle = parseResponseBundle;
const openuiResponseBundleSpec = openuiSpec([openuiResponseText, openuiBool]);
parseResponseBundle = function (...args) {
  return openuiCall(openuiResponseBundleSpec, undefined, undefined, args, converted => openuiApply(openuiParseResponseBundle, undefined, converted));
};

// CommonJS exports captured the unguarded values; web exports are live
// bindings. Refuse to load if the package exports anything not guarded here.
if (typeof exports !== 'undefined') {
  const openuiExports = { RuntimeSession, EvaluationHandle, ActionHandle, ContinuationHandle,
    ParsedProgram, EvaluationOutput, ActionOutput, ReplacementOutput, RuntimeLimits, ParseLimits, ComponentLibrary,
    StreamSession, parse, parseProgram, parseMessage, buildMessage, parseResponseBundle };
  for (const key of openuiOwnKeys(exports)) {
    if (key !== '__esModule' && !openuiHasOwn(openuiExports, key)) throw openuiUncovered('exports', key);
  }
  Object.assign(exports, openuiExports);
}
