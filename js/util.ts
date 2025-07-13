export function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max);
}

// Formats a progress percentage as a string, and prevents partial matches from being rounded to 0.00% or 100.00%.
export function formatPercent(value: number) {
  if (value !== 0.0 && value !== 100.0) {
    value = clamp(value, 0.01, 99.99);
  }
  return `${value.toFixed(2)}%`;
}
