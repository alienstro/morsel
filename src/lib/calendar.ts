import type { CalendarEvent } from "./api";

/** The name of the video app of a meeting link. */
export function meetingPlace(link: string | null): string | null {
  if (!link) return null;
  let host: string;
  try {
    host = new URL(link).hostname;
  } catch {
    return null;
  }
  if (host.endsWith("meet.google.com")) return "Google Meet";
  if (host.endsWith("zoom.us")) return "Zoom";
  if (host.endsWith("teams.microsoft.com") || host.endsWith("teams.live.com")) return "Microsoft Teams";
  return null;
}

/** "Sam", "Sam and Priya", "Sam, Priya and Leo". */
export function joinNames(names: string[]): string {
  if (names.length <= 1) return names.join("");
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

/** "Google Meet with Sam and Priya", "Zoom", "With Sam", or "". */
export function eventWho(e: CalendarEvent): string {
  const place = meetingPlace(e.link);
  const people = joinNames(e.people);
  if (place && people) return `${place} with ${people}`;
  return place ?? (people ? `With ${people}` : "");
}

/** "Google Meet with Sam and Priya, in 6 min". The first letter is a capital. */
export function eventLine(e: CalendarEvent, now: number): string {
  const who = eventWho(e);
  const mins = Math.ceil((e.start - now) / 60_000);
  const when = e.start <= now ? "now" : mins < 60 ? `in ${mins} min` : "";
  const line = [who, when].filter(Boolean).join(", ");
  return line.charAt(0).toUpperCase() + line.slice(1);
}
