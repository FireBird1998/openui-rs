# The chat app

A sandboxed Mac app that sends the open thread to the Thesys Gateway's Chat
Completions endpoint and draws each answer natively as it streams. It asks for
react-ui's library in cloud prompt mode by default, as upstream's
`templates/openui-cloud` does. The core generates the cloud config block
and the full system prompt from react-ui's library and prompt options.
Why it is SwiftUI on the Swift binding:
[ADR 0018](../../docs/adr/0018-the-chat-app-is-swiftui-on-the-swift-binding.md).
The plan for the rest is spec #129.

The **Threads** sidebar lists this app's conversations in Thesys, newest first.
Select a thread to reopen it, or use **New Chat** to start a blank one. The first
message supplies the title. The row's actions open a rename dialog or delete
the thread immediately. Deleting the open thread returns to a blank chat.
A new thread appears in the sidebar once its first turn is saved. Failed
turns show an error and leave the saved transcript in place. If an append's
result is uncertain, the app reads the stored items before sending another
message.

## Run it with your key

1. Build and open it:

   ```bash
   scripts/chat.sh run
   ```

   This builds the core's binding, then the app, wraps it as
   `target/chat/debug/Chat.app`, signs it ad hoc for this Mac with the
   sandbox entitlements in `App/Chat.entitlements` (network client and server),
   and opens it.
2. Open **Chat > Settings…** (⌘,), paste your Thesys API key into the field
   and press **Save**. The key goes into your login Keychain under the
   service `dev.openui-rs.chat`; the field is cleared and the key is never
   shown again. **Remove Key** deletes it.
3. Type a message and press Return (Shift-Return for a new line), or click a
   starter. While an answer runs, the send button is a stop button.
4. Use the toolbar to select a model or switch the theme. The model
   selection is shared and saved. The app follows macOS appearance until you
   flip the toggle, then remembers the chosen light or dark theme.
5. Choose **Cloud prompt** or **Full prompt** in **Chat > Settings…**.
   The choice applies to the next send in any chat window. Each launch
   starts in cloud prompt mode.

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
   error exactly. An explicit model refusal selects and remembers
   `google/gemini-3.6-flash-free` for the next send. The callout stays
   visible until you send again.
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
   such as an unknown model, inside a successful stream). A failed request
   restores the thread to its committed turns.
6. **A follow-up.** After an answer, send "Make it shorter". Does the second
   answer take the first into account?
7. **Actions.** Click a button or submit a form in a new or reopened answer.
   Check that a reply streams and the user bubble shows the action label.
   A submission with entered fields shows a collapsed **Form data**
   disclosure. After the next reply finishes saving, use a control in an
   earlier answer. Absolute HTTP and HTTPS links open in the default
   browser. If an action reply fails, the callout shows the reason and
   the thread returns to its pre-action turns.

If anything fails without a message, open Console.app, filter on the process
`Chat`, and send what it shows around the failure (the app logs no requests
and no key).

### Notion sign-in

Chat reuses a dynamic client registration with its issuing authorization
server and original loopback redirect. A failed Keychain write keeps the
registration in memory for a retry while Chat is running. An expired client
secret requires a replacement, and the previous saved grant stays in place
until that replacement signs in and saves successfully.
[MCP client registration](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration#authorization-server-binding) ·
[Swift SDK 0.12.1 registration handling](https://github.com/modelcontextprotocol/swift-sdk/blob/a0ae212ebf6eab5f754c3129608bc5557637e605/Sources/MCP/Base/Authorization/OAuthAuthorizer.swift#L673-L697) ·
[RFC 7591 registration response](https://www.rfc-editor.org/rfc/rfc7591.html#section-3.2.1)

Chat saves a completed OAuth grant before fetching tools so a failed tools
request can be retried without signing in again. If Settings refuses a
newly added server's tool list, Chat clears its saved grant because the
server has no Settings entry. The registration stays in memory for the
next attempt. If a registration response is lost, Chat has
no issued client ID to retain. RFC 7591 returns that ID in the successful
response. [RFC 7591 registration response](https://www.rfc-editor.org/rfc/rfc7591.html#section-3.2.1)

Use your own Notion account in the browser. Report the app's status or
error text and the tool count or a few tool names. Keep callback URLs, codes,
keys, tokens and account details private.

- [ ] Open **Chat > Settings… > MCP Servers**. Enter
  `https://mcp.notion.com/mcp` and click **Connect**. Confirm the browser
  opens Notion, lets you grant access to the intended workspace, then
  shows OpenUI Chat's loopback confirmation page. Report where the flow
  stops if you do not see that page.
- [ ] Expand the server in Settings. Check which tools are listed and
  that their switches start on. Turn one off, quit and reopen Chat,
  then use **Refresh Tools**. Report whether the tool list refreshes
  with that switch still off and whether the browser opens again.
- [ ] After the access token expires, use **Refresh Tools** again. Report
  whether the tools refresh without a browser. If refresh fails, report
  the message, whether the previous tools and switches stay visible, and
  whether **Sign In Again** completes.
- [ ] Check that the list loads despite Notion returning HTTP 405 to a
  standalone GET. In SDK 0.12.1 the optional standalone SSE GET stops
  on 405. Chat disables that GET and accepts JSON or SSE response bodies
  on Streamable HTTP POSTs. The loopback test verifies listing against a
  fixture with that 405 and a tools page returned as POST SSE.
- [ ] Use **Remove** if you want to delete this server and its login
  from Chat. Report whether adding it again repeats browser sign-in.
- [ ] Verify the real Keychain store. After connecting, locate the generic
  password item for the server under the `dev.openui-rs.chat` service.
  Use **Remove** and confirm the same item disappears from the Keychain.
  Keep the item's contents private.
- [ ] Inspect the signed `Chat.app` bundle with
  `codesign -d --entitlements :- /path/to/Chat.app`. Confirm exactly
  three entitlements are true: `com.apple.security.app-sandbox`,
  `com.apple.security.network.client`, and
  `com.apple.security.network.server`.

See [Notion’s MCP client guide](https://developers.notion.com/guides/mcp/build-mcp-client)
for its discovery and refresh behavior. The checklist requires a live
Notion account. The local tests use synthetic credentials.

## How it is built and tested

- `Sources/ChatKit/KeyStore.swift`: the key store interface and its Keychain
  implementation (a generic password item, no access group).
- `Sources/ChatKit/Gateway.swift`: the Gateway client interface and its
  implementation over an HTTP transport, reading OpenAI-format server-sent
  events.
- `Sources/ChatKit/ChatThread.swift`: one thread, sending, stopping, errors,
  and drawing each answer in a `LiveProgram`.
- `Sources/ChatKit/Conversations.swift`: the Conversations client. It uses a
  frontend token scoped to `demo-user` and `openui-mac-chat` for the sidebar
  and a bearer-key request to append each turn. The workspace reconstructs
  messages from the stored items when a thread is opened.
- `Sources/ChatKit/Library/`: copies of the corpus's react-ui JSON Schema and
  prompt spec (a test keeps them equal to `conformance/`).
- `Tests/ChatKitTests/`: tests with an in-memory key store and recorded
  exchanges in `Recordings/`. Those recordings are shaped by hand in OpenAI's
  format until a real run supplies real ones.

```bash
scripts/chat.sh test       # the binding, then the chat app's tests
```

## MCP dependencies

The app uses the official MCP Swift SDK 0.12.1. The SDK's code is in an MIT
to Apache-2.0 licensing transition. Its CC-BY-4.0 documentation is outside
the shipped app.

| Package | Resolved version | Licence in the pinned checkout | Link status |
| --- | --- | --- | --- |
| swift-system | 1.4.0 | Apache-2.0 with Swift Runtime Library Exception (`LICENSE.txt`) | Linked |
| swift-log | 1.6.2 | Apache-2.0 (`LICENSE.txt`) | Linked |
| eventsource | 1.1.0 | Apache-2.0 (`LICENSE`) | Linked |
| swift-atomics | 1.3.0 | Apache-2.0 with Swift Runtime Library Exception (`LICENSE.txt`) | Resolution only |
| swift-collections | 1.3.0 | Apache-2.0 with Swift Runtime Library Exception (`LICENSE.txt`) | Resolution only |
| swift-nio | 2.94.0 | Apache-2.0 (`LICENSE.txt`) | Resolution only |

SwiftPM resolved only swift-atomics, swift-collections and swift-nio in
addition to the four linked packages. The linkage audit verifies that the
Chat executable contains none of those three resolution-only packages.

`scripts/check-viewer.sh` checks the package pins, produces the Chat
executable's linker map, and checks its static inputs and Mach-O load
commands. After building the binding, run just that audit with
`python3 scripts/check-chat-linkage.py` (pass `--disable-sandbox` if needed
in a nested sandbox).

## Request and response limits

The session's program-size limit (1 MiB by default) ends an answer that
grows past it. The app cancels the request and shows the session's message
in the thread's error callout. The network clients apply the following bounds.

- `ThesysGateway.maxLineBytes`, 1 MiB: the longest server-sent-event line the
  app buffers. It equals the core's program-size limit; a longer line is
  refused with "The Gateway sent a line longer than 1 MiB."
- `ThesysGateway.maxErrorBodyBytes`, 64 KiB: how much of an HTTP error body is
  read for its message; the rest is not read.
- Conversations uses OpenUINative's default host input byte limit
  (`maxJsonBytes`) for each JSON response. The sidebar requests the first
  100 conversations, and reopening follows up to 50 item pages.

- MCP bounds each successful HTTP response to the same host input limit,
  each error body to the Gateway's error-body cap, and each tools listing to
  32 pages and 1,024 tool names. An HTTP request has a 30-second resource
  timeout and a browser hand-back has two minutes from opening the browser.
  Callback listener shutdown allows one second (`LoopbackHTTPListener.shutdownDeadline`)
  for its local close and acknowledgement. If that deadline expires, Chat
  closes the native socket directly and reports "The sign-in listener took too
  long to stop. Its port was closed. Try again." The port is free at once, so
  sign-in can be retried immediately. No SDK, specification or upstream value
  exists for this bound. Measured over 1,000 closes each on arm64 macOS 27.2,
  a normal shutdown took a median of 41 µs idle and 68 µs under load, and at
  most 0.756 ms idle and 4.8 ms under load. The maintainer approved this
  one-second value on 2026-11-25.
