/** Trims a trailing ".0" so "1.0K" reads as "1K". */
function trimTrailingZero(value: string): string {
  return value.replace(/\.0$/, "");
}

export function formatTokenValue(tokens: number): string {
  if (!Number.isFinite(tokens) || tokens <= 0) {
    return "0";
  }
  if (tokens >= 1_000_000_000) {
    return `${trimTrailingZero((tokens / 1_000_000_000).toFixed(1))}B`;
  }
  if (tokens >= 1_000_000) {
    return `${trimTrailingZero((tokens / 1_000_000).toFixed(1))}M`;
  }
  if (tokens >= 1_000) {
    return `${trimTrailingZero((tokens / 1_000).toFixed(1))}K`;
  }
  return String(Math.round(tokens));
}

/** Drops the smaller unit once it is zero, so "2h" beats "2h 0m". */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) {
    return "0s";
  }
  if (ms < 1_000) {
    return "<1s";
  }
  const totalSeconds = Math.floor(ms / 1_000);
  if (totalSeconds < 60) {
    return `${totalSeconds}s`;
  }
  const minutes = Math.floor(totalSeconds / 60);
  if (minutes < 60) {
    const seconds = totalSeconds % 60;
    return seconds > 0 ? `${minutes}m ${seconds}s` : `${minutes}m`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    const remainingMinutes = minutes % 60;
    return remainingMinutes > 0 ? `${hours}h ${remainingMinutes}m` : `${hours}h`;
  }
  const days = Math.floor(hours / 24);
  const remainingHours = hours % 24;
  return remainingHours > 0 ? `${days}d ${remainingHours}h` : `${days}d`;
}

/** Per-request costs are small, so keep four decimals rather than currency rounding. */
export function formatCost(usd: number): string {
  if (!Number.isFinite(usd) || usd <= 0) {
    return "$0";
  }
  return `$${usd.toFixed(4)}`;
}
