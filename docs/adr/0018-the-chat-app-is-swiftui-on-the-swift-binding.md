---
status: accepted
---

# The chat app is SwiftUI on the Swift binding

Decided by the maintainer in spec #129 (2026-11-21), built from #150. This
record says why, and what the first version departs from.

## What happened

The maintainer wants a Mac app he can chat with: a model answers in OpenUI
Lang, the answer is drawn natively, and later its buttons, forms and real
tools (Notion first) work. Nothing in the repository exercised the Swift
binding the way a product would; the Viewer is a development tool that
replays programs and offers no tools.

## The decision

The chat app is a new SwiftUI app for macOS 27 in `tools/chat/`, beside the
Viewer, with its own Swift package and tests. It draws and reads the language
only through `OpenUINative`'s public interface; it does not import the
generated binding. A gap it finds is fixed in OpenUINative, not worked around
in the app.

Tauri (a web view with upstream's React renderer) and GPUI (a Rust UI) were
rejected. Either would make a chat app, but neither exercises the Swift
binding, which is the point: the app is a real product's test of
OpenUINative, so that the next Swift app does not meet the gaps first.

## Departures and limits, recorded

- **The key calls the Gateway directly.** Thesys's docs advise keeping the API
  key on a server. The app keeps it in the login Keychain and sends it as a
  bearer token to `api.thesys.dev` itself. Accepted because the app is
  personal and not distributed. The key enters through Settings only and never
  passes through a log, a file or a test; tests use a key store in memory.
- **Sandboxed, signed for this Mac only.** App Sandbox with outgoing network
  access, signed ad hoc by `scripts/chat.sh`. There is no paid developer
  account, so nothing depends on one: no Keychain access group, no associated
  domains. The key's Keychain item belongs to the app's signature, so macOS
  may ask again after a rebuild.
- **react-ui first.** The first version asks for react-ui's library in cloud
  prompt mode and draws with the components OpenUINative already has, as
  upstream's openui-cloud template does (#129, amendment 1). The native
  Intelligent UI library becomes a choice when it lands.
- **Shapes not yet seen.** The Gateway's streamed chunks and errors are read
  as OpenAI's Chat Completions format, which Thesys's docs say the Gateway
  implements; the first real run with the maintainer's key confirms or
  corrects that, and its recordings replace the hand-shaped ones.

## Consequences

The glossary gains **Chat app**. `scripts/check-viewer.sh` builds and tests
the chat app with the package and the Viewer.

(2026-11-25: since #153, cloud prompt mode starts each launch, Settings offers full prompt mode, and the model and theme selections persist as app preferences.)
