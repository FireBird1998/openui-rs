# Scripted tool outcomes

In **Open a Program**, paste or choose the program, select its component library, then use **Choose a Tool Script…** to select a UTF-8 JSON file. Choose **Open**. To try the bundled program and script together, choose **Open News Desk Sample** in the same sheet with the `bench` library available.

**Response Samples** includes **Message** and **Response Bundle**. The message draws “Morning edition”. In the bundle, the Viewer’s tool script answers the `edition` query with “Evening edition”. The Cloud script code stays in the raw response, which the source pane shows as you step through it.

A script maps each tool name to a nonempty array of outcomes. For example, this desk fails on its first call and returns an object on retry.

```json
{
  "desk": [
    {"delayMs": 600, "failed": "Desk is offline"},
    {"delayMs": 600, "value": {"connected": true}}
  ],
  "publish": [{"value": null}]
}
```

| Outcome field | What the tool provider reports |
| --- | --- |
| `value` | A JSON result, including `null` |
| `error` | The tool reported the given text (`mcp-error`) |
| `failed` | The call failed with the given text (`tool-error`) |

Give each outcome exactly one of these fields. Optional `delayMs` waits a whole number of milliseconds from 0 through 2147483647 and defaults to zero. Each tool appears once. An unscripted tool produces `tool-not-found` with the available names in file order.

Calls to the same tool share one cursor across all argument values. Each call reserves an outcome before waiting for its delay, so another call can finish first. Each reservation advances the cursor up to the last outcome, which repeats. Pausing queries cancels calls in flight and leaves the cursors in place. After resume, the next call takes the outcome at that tool’s cursor, which already accounts for every outcome reserved before the pause. Loading the program into a fresh runtime restarts the sequences.

## Try the News Desk

1. Watch the first headline load and the desk fail. The Queries pane shows statement values, loading and refetching IDs, and errors. Choose **Retry failed queries** to see “Desk connected”.
2. Leave the drawing open for the eight-second refresh interval. The previous headline stays visible while the next edition loads. **Refresh headline** also runs the query.
3. Choose **Publish edition**. Inspect the `save` mutation as it goes from loading to success. Its action then increments `$published` in the store and refetches the headline.

The sample files are [news-desk.oui](Sources/ViewerKit/Samples/news-desk.oui) and [news-desk.tools.json](Sources/ViewerKit/Samples/news-desk.tools.json).
