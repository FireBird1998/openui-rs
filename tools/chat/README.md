# The chat app

A sandboxed Mac app that sends one thread to the Thesys Gateway's Chat
Completions endpoint and draws each answer natively as it streams. It asks for
react-ui's library in cloud prompt mode, as upstream's `templates/openui-cloud`
does: the system message is the cloud config block the core builds from
react-ui's library and prompt options. Why it is SwiftUI on the Swift binding:
[ADR 0018](../../docs/adr/0018-the-chat-app-is-swiftui-on-the-swift-binding.md).
The plan for the rest is spec #129.

## Run it with your key

1. Build and open it:

   ```bash
   scripts/chat.sh run
   ```

   This builds the core's binding, then the app, wraps it as
   `target/chat/debug/Chat.app`, signs it ad hoc for this Mac with the
   sandbox entitlements in `App/Chat.entitlements` (outgoing network only),
   and opens it.
2. Open **Chat > Settings…** (⌘,), paste your Thesys API key into the field
   and press **Save**. The key goes into your login Keychain under the
   service `dev.openui-rs.chat`; the field is cleared and the key is never
   shown again. **Remove Key** deletes it.
3. Type a message and press Return (Shift-Return for a new line), or click a
   starter. While an answer runs, the send button is a stop button.

After a rebuild the app's ad hoc signature changes, so macOS may ask once for
your login password before the app reads the key: choose **Always Allow**.

## What to try and what to report

Please report back, for each item, what you saw. Copy the Gateway's message
from the red callout when there is one (it is selectable). No key appears in
it: the app takes the key out of any message it shows.

1. **A first message.** Click "Compare breakfast nutrition". Does an answer
   stream in and draw (a table, a chart, cards)? If nothing draws, is the
   bubble empty (an answer with no OpenUI Lang program draws as nothing, as
   in the template)?
2. **The default model.** The app asks for `anthropic/claude-sonnet-5.5`.
   Does the Gateway accept it, or does the callout show an error? Quote the
   error exactly. (If it is refused, the default becomes the template's
   `google/gemini-3.6-flash-free` in the next ticket.)
3. **How cloud mode treats the library.** The system message names react-ui's
   library and its examples but not its components (that is what the cloud
   config block is). Do the answers use react-ui's components (`Stack`,
   `Card`, `Table`, `BarChart`, `Form`…) and draw, or do they use names the app
   does not draw (shown as labelled boxes), or plain prose?
4. **Stop.** Send "Help me pick a car" and press stop while it streams. Does
   the partial answer stay, with no error?
5. **The error shape.** In Settings, save a deliberately wrong key (for
   example `not-a-key`), send a message, and quote the callout. Then save
   your real key again. Say whether the error came at once or after the
   answer had started (the cookbook says the Gateway reports some errors,
   such as an unknown model, inside a successful stream).
6. **A follow-up.** After an answer, send "Make it shorter". Does the second
   answer take the first into account?

If anything fails without a message, open Console.app, filter on the process
`Chat`, and send what it shows around the failure (the app logs no requests
and no key).

## How it is built and tested

- `Sources/ChatKit/KeyStore.swift`: the key store interface and its Keychain
  implementation (a generic password item, no access group).
- `Sources/ChatKit/Gateway.swift`: the Gateway client interface and its
  implementation over an HTTP transport, reading OpenAI-format server-sent
  events.
- `Sources/ChatKit/ChatThread.swift`: one thread, sending, stopping, errors,
  and drawing each answer in a `LiveProgram`.
- `Sources/ChatKit/Library/`: copies of the corpus's react-ui JSON Schema and
  prompt spec (a test keeps them equal to `conformance/`).
- `Tests/ChatKitTests/`: tests with an in-memory key store and recorded
  exchanges in `Recordings/`. Those recordings are shaped by hand in OpenAI's
  format until a real run supplies real ones.

```bash
scripts/chat.sh test       # the binding, then the chat app's tests
```

## Limits the app sets itself

The app adds no limit on an answer's size: the session's own program-size
limit (1 MiB by default) ends an answer that grows past it, and the app shows
the session's message under the answer and cancels the request. Two bounds
cover bytes the session never sees:

- `ThesysGateway.maxLineBytes`, 1 MiB: the longest server-sent-event line the
  app buffers. It equals the core's program-size limit; a longer line is
  refused with "The Gateway sent a line longer than 1 MiB."
- `ThesysGateway.maxErrorBodyBytes`, 64 KiB: how much of an HTTP error body is
  read for its message; the rest is not read.

Both are safety bounds, not spend guards, and can be changed.
