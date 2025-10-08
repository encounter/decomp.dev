export function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max);
}

// Formats a progress percentage as a string, and prevents partial matches from being rounded to 0.00% or 100.00%.
export function formatPercent(value: number) {
  let clamped = value;
  if (clamped !== 0.0 && clamped !== 100.0) {
    clamped = clamp(clamped, 0.01, 99.99);
  }
  return `${clamped.toFixed(2)}%`;
}
