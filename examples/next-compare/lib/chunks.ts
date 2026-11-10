/** `text` cut into pieces of `size` code points (never inside a surrogate pair). */
export function chunk(text: string, size: number): string[] {
  const points = Array.from(text);
  const pieces: string[] = [];
  for (let i = 0; i < points.length; i += size) pieces.push(points.slice(i, i + size).join(""));
  return pieces;
}

export const CHUNK_SIZES = [1, 4, 16, 64] as const;
