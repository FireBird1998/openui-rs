import { Compare } from "../components/Compare.tsx";

export default function Page() {
  return (
    <main>
      <header>
        <h1>lang-core vs openui-rs</h1>
        <p>
          The same stream of OpenUI Lang, chunk for chunk, into two copies of upstream’s React <code>&lt;Renderer&gt;</code> with
          react-ui’s component library. The only difference is the parser behind <code>createStreamingParser</code>: upstream’s
          JavaScript on the left, this repo’s Rust core compiled to WebAssembly on the right.
        </p>
      </header>
      <Compare />
    </main>
  );
}
