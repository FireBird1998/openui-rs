import Testing

/// Async UI helpers suspend instead of starting nested main run loops. Keep
/// the windows serialized so their focus and redraws do not overlap.
@Suite(.serialized) struct NativeUITests {}
