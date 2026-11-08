export type NotePart = { text: string; yours: boolean };

/** Lower case, outer spaces, inner runs of spaces, and list or checkbox marks do not count. */
function normalize(line: string): string {
  return line
    .replace(/^\s*(?:[-*+]\s+)?(?:\[[ xX]\]\s+)?/, "")
    .trim()
    .replace(/\s+/g, " ")
    .toLowerCase();
}

/** Splits the note into lines. A line is "yours" when it matches a line of the rough notes. */
export function markYours(noteMd: string, userNotes: string): NotePart[] {
  const mine = new Set(userNotes.split("\n").map(normalize).filter((l) => l.length > 0));
  return noteMd.split("\n").map((text) => {
    const key = normalize(text);
    return { text, yours: key.length > 0 && mine.has(key) };
  });
}
