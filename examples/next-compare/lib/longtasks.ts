/** Long tasks (over 50 ms on the main thread) seen between start and stop: page-wide, not per pane. */
export interface LongTasks {
  count: number;
  totalMs: number;
}

export function watchLongTasks(): { stop(): LongTasks | null } {
  if (typeof PerformanceObserver === "undefined" || !PerformanceObserver.supportedEntryTypes?.includes("longtask")) {
    return { stop: () => null };
  }
  const durations: number[] = [];
  const observer = new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) durations.push(entry.duration);
  });
  observer.observe({ type: "longtask" });
  return {
    stop() {
      for (const entry of observer.takeRecords()) durations.push(entry.duration);
      observer.disconnect();
      return { count: durations.length, totalMs: durations.reduce((a, b) => a + b, 0) };
    },
  };
}
