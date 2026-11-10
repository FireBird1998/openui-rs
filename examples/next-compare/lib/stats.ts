/** Count, mean, p95, max and total of a list of milliseconds. */
export interface Summary {
  count: number;
  mean: number;
  p95: number;
  max: number;
  total: number;
}

export function summarize(ms: readonly number[]): Summary {
  if (ms.length === 0) return { count: 0, mean: 0, p95: 0, max: 0, total: 0 };
  const sorted = [...ms].sort((a, b) => a - b);
  const total = ms.reduce((a, b) => a + b, 0);
  return {
    count: ms.length,
    mean: total / ms.length,
    p95: sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * 0.95) - 1)],
    max: sorted[sorted.length - 1],
    total,
  };
}
