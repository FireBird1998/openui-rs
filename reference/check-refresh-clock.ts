// From reference/: bun --no-install check-refresh-clock.ts
import assert from 'node:assert/strict';
import { RefreshClock } from './refresh-clock';

async function measure(blockMs: number) {
  const period = 100;
  const wakes: number[] = [];
  const clock = new RefreshClock();
  let fired = 0;
  clock.setInterval(() => fired++, period);
  const start = performance.now();
  let interval: ReturnType<typeof setInterval> | undefined;
  let watchdog: ReturnType<typeof setTimeout> | undefined;
  try {
    await new Promise<void>((resolve, reject) => {
      watchdog = setTimeout(() => reject(Error('three interval callbacks timed out')), 3000);
      interval = setInterval(() => {
        wakes.push(Math.round(performance.now() - start));
        if (wakes.length === 3) resolve();
      }, period);
      if (blockMs) Bun.sleepSync(blockMs);
    });
  } finally {
    clearInterval(interval);
    clearTimeout(watchdog);
  }
  let previous = 0;
  for (const [i, wake] of wakes.entries()) {
    // Allow 3 ms for timer resolution and rounding the time observations.
    assert.ok(wake >= previous + period - 3, JSON.stringify(wakes));
    if (i === 0) {
      if (blockMs) assert.ok(wake >= blockMs - 1);
      else assert.ok(wake < 2 * period, 'on-time sample missed its first period');
    }
    const now = Math.max(wake, clock.nextDue!);
    clock.advance(now);
    assert.equal(fired, i + 1);
    assert.equal(clock.nextDue, now + period);
    previous = wake;
  }
  console.log(`Bun ${Bun.version}, block ${blockMs} ms: ${wakes.join(', ')} ms`);
}

await measure(0);
await measure(350);
