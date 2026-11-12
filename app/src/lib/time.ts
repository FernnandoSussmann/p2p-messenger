const LIMIT_MS = 8.64e15;

const valid = (ms: number) => Number.isFinite(ms) && Math.abs(ms) <= LIMIT_MS;

export function formatLocalTime(ms: number): string {
  if (!valid(ms)) return "unknown time";
  return new Intl.DateTimeFormat(undefined, { dateStyle: "short", timeStyle: "short" }).format(
    new Date(ms),
  );
}

export function guardedIso(ms: number): string | undefined {
  return valid(ms) ? new Date(ms).toISOString() : undefined;
}
