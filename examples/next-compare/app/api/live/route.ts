// Live: streams a model's answer as plain text. The model is any
// OpenAI-compatible chat-completions endpoint, configured only by the
// server's environment (.env.local): LLM_BASE_URL, LLM_API_KEY, LLM_MODEL.
// The system prompt is react-ui's library prompt, the one its examples use
// (openuiLibrary.prompt(openuiPromptOptions)), written by scripts/inputs.mjs.
import { prompt as systemPrompt } from "../../../lib/system-prompt.generated.json";

export const dynamic = "force-dynamic";

function config() {
  const { LLM_BASE_URL: baseUrl, LLM_API_KEY: apiKey, LLM_MODEL: model } = process.env;
  return baseUrl && apiKey && model ? { baseUrl: baseUrl.replace(/\/+$/, ""), apiKey, model } : null;
}

/** Whether Live is set up (never the values). */
export function GET() {
  return Response.json({ configured: config() !== null });
}

export async function POST(request: Request) {
  const settings = config();
  if (!settings) return new Response("Live isn't configured: set LLM_BASE_URL, LLM_API_KEY and LLM_MODEL in .env.local.", { status: 503 });
  const { prompt } = (await request.json().catch(() => ({}))) as { prompt?: unknown };
  if (typeof prompt !== "string" || !prompt.trim()) return new Response("No prompt.", { status: 400 });

  const upstream = await fetch(`${settings.baseUrl}/chat/completions`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${settings.apiKey}` },
    body: JSON.stringify({
      model: settings.model,
      stream: true,
      messages: [
        { role: "system", content: systemPrompt },
        { role: "user", content: prompt },
      ],
    }),
    signal: request.signal,
  }).catch((e: unknown) => new Response(e instanceof Error ? e.message : String(e), { status: 502 }));
  if (!upstream.ok || !upstream.body) {
    const detail = (await upstream.text().catch(() => "")).slice(0, 500);
    return new Response(`The model endpoint answered ${upstream.status}: ${detail}`, { status: 502 });
  }

  // Server-sent events in, the content deltas out as text.
  let buffered = "";
  const deltas = new TransformStream<string, string>({
    transform(text, out) {
      buffered += text;
      const lines = buffered.split("\n");
      buffered = lines.pop() ?? "";
      for (const line of lines) {
        const data = line.startsWith("data:") ? line.slice(5).trim() : "";
        if (!data || data === "[DONE]") continue;
        try {
          const content = JSON.parse(data)?.choices?.[0]?.delta?.content;
          if (typeof content === "string" && content) out.enqueue(content);
        } catch {
          // A line that isn't JSON (a comment or keep-alive): skip it.
        }
      }
    },
  });
  return new Response(upstream.body.pipeThrough(new TextDecoderStream()).pipeThrough(deltas).pipeThrough(new TextEncoderStream()), {
    headers: { "content-type": "text/plain; charset=utf-8", "cache-control": "no-store" },
  });
}
