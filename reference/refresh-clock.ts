// Deterministic platform timer queue. Execute the real manager callbacks in
// deadline/installation order. Only the script can settle a provider promise.
export class RefreshClock {
  constructor(private readonly minimumMs?: number) {}

  private now = 0;
  private serial = 0;
  private timers = new Map<number, { tick: () => void; period: number; due: number }>();

  setInterval = (tick: () => void, delay: number) => {
    const period = delay >= 1 && delay <= 0x7fffffff ? Math.trunc(delay) : 1;
    const id = ++this.serial;
    // Optional spec replay. Only the platform delay differs; the manager,
    // promises and script are identical to the unmodified reference replay.
    if (this.minimumMs === undefined || delay !== Infinity) {
      const effective = Math.max(period, this.minimumMs ?? 1);
      this.timers.set(id, { tick, period: effective, due: this.now + effective });
    }
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
    for (;;) {
      const due = this.nextDue;
      if (due === null || due > now) break;
      this.now = due;
      for (const timer of this.timers.values()) {
        if (timer.due !== due) continue;
        timer.due += timer.period;
        timer.tick();
      }
    }
    this.now = now;
  }
}
