const DAY_MS = 86_400_000;

function startOfDay(ms: number): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

function time(ms: number): string {
  return new Date(ms).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit", hour12: false });
}

/** How many calendar days `ms` is before `now`: 0 for today, 1 for yesterday. */
export function daysAgo(ms: number, now: number = Date.now()): number {
  return Math.round((startOfDay(now) - startOfDay(ms)) / DAY_MS);
}

/** The day of a meeting: "Today", "Yesterday", a weekday in the last week, else "Mon 2 Nov". */
export function dayLabel(ms: number, now: number = Date.now()): string {
  const ago = daysAgo(ms, now);
  if (ago === 0) return "Today";
  if (ago === 1) return "Yesterday";
  const d = new Date(ms);
  if (ago > 1 && ago < 7) return d.toLocaleDateString("en-GB", { weekday: "long" });
  return shortDate(ms);
}

/** "Mon 2 Nov". */
export function shortDate(ms: number): string {
  const d = new Date(ms);
  const weekday = d.toLocaleDateString("en-GB", { weekday: "short" });
  const month = d.toLocaleDateString("en-GB", { month: "short" });
  return `${weekday} ${d.getDate()} ${month}`;
}

/** "Today, 10:00", "Yesterday, 16:30" or "Mon 2 Nov, 09:30". */
export function formatWhen(ms: number, now: number = Date.now()): string {
  const ago = daysAgo(ms, now);
  const day = ago === 0 ? "Today" : ago === 1 ? "Yesterday" : shortDate(ms);
  return `${day}, ${time(ms)}`;
}

/** The length of a meeting in whole minutes (at least 1), or "—" when it has no end. */
export function meetingLength(startedAt: number, endedAt: number | null): string {
  if (endedAt === null || endedAt < startedAt) return "—";
  return `${Math.max(1, Math.round((endedAt - startedAt) / 60_000))} min`;
}

export { time as clockTime };
