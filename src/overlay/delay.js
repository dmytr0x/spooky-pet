const REF_CHARS = 80;
const VISIBLE_MIN_MS = 1000;

export function chunkVisibleMs(text, baseDelayMs) {
  const base = Math.round(Number(baseDelayMs));
  if (!Number.isFinite(base) || base <= 0) {
    return VISIBLE_MIN_MS;
  }
  const n = String(text ?? "").trim().length;
  if (n <= 0) {
    return Math.min(base, VISIBLE_MIN_MS);
  }
  const scaled = Math.round((base * n) / REF_CHARS);
  return Math.min(base, Math.max(VISIBLE_MIN_MS, scaled));
}
