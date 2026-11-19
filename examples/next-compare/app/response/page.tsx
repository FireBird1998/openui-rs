"use client";

import { Renderer } from "@openuidev/react-lang";
import { openuiLibrary } from "@openuidev/react-ui/genui-lib";
import { useEffect, useState } from "react";
import { loadWasm } from "../../lib/engines.ts";
import { response, tools } from "../../lib/response-example.ts";
import { ComponentLibrary, RuntimeSession, type EvaluationOutput } from "../../wasm/openui_wasm.js";

export default function ResponseComparison() {
  const [value, setValue] = useState("");
  useEffect(() => {
    let closed = false;
    let session: RuntimeSession | undefined;
    const show = (evaluation: EvaluationOutput) => {
      try { setValue(JSON.parse(evaluation.json).root?.props.text ?? ""); }
      finally { evaluation.free(); }
    };
    loadWasm().then(() => {
      if (closed) return;
      const library = new ComponentLibrary(JSON.stringify(openuiLibrary.toJSONSchema()));
      try { session = new RuntimeSession(library, JSON.stringify(openuiLibrary.toSpec())); }
      finally { library.free(); }
      const current = session;
      current.setToolProvider(tools, {onChange: () => show(current.evaluate())});
      show(current.setResponse(response, false).evaluation);
    }).catch(error => { if (!closed) setValue(String(error)); });
    return () => { closed = true; session?.free(); };
  }, []);
  return (
    <main>
      <h1>Response bundle comparison</h1>
      <h2>React Renderer</h2>
      <Renderer response={response} isStreaming={false} library={openuiLibrary} toolProvider={tools} />
      <h2>WebAssembly driver</h2>
      <output>{value}</output>
    </main>
  );
}
