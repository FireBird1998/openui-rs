// Deterministic platform timer queue. Execute the real manager callbacks in
// deadline/installation order. Only the script can settle a provider promise.
export class RefreshClock {
  private now = 0;
  private serial = 0;
  private timers = new Map<number, { tick: () => void; period: number; due: number }>();

  setInterval = (tick: () => void, delay: number) => {
    const period = delay >= 1 && delay <= 0x7fffffff ? Math.trunc(delay) : 1;
    const id = ++this.serial;
    this.timers.set(id, { tick, period, due: this.now + period });
    return id;
  };
  clearInterval = (id: number) => { this.timers.delete(id); };

  get nextDue(): number | null {
    let earliest = Infinity;
    for (const t of this.timers.values()) earliest = Math.min(earliest, t.due);
    return earliest === Infinity ? null : earliest;
  }

  advance(now: number) {
    if (!Number.isSafeInteger(now) || now < this.now) throw Error('invalid scripted time');
    this.now = now;
    const ready = [...this.timers.entries()].filter(([, t]) => t.due <= now);
    ready.sort(([, a], [, b]) => a.due - b.due);
    for (const [id, timer] of ready) {
      if (!this.timers.has(id)) continue;
      timer.due = now + timer.period;
      timer.tick();
    }
  }
}
