//! Cuts a meeting into overlapping parts for search.

use crate::db::parts::NewPart;
use crate::notes::prompt::transcript_text;
use crate::types::TranscriptLine;

/// The largest part, in estimated tokens (characters ÷ 4).
pub const PART_TOKENS: usize = 400;
/// The text that each part repeats from the end of the part before it.
pub const OVERLAP_TOKENS: usize = 50;

const PART_CHARS: usize = PART_TOKENS * 4;
const OVERLAP_CHARS: usize = OVERLAP_TOKENS * 4;

/// One line of text with its times, before the lines are packed into parts.
struct Unit {
    text: String,
    chars: usize,
    start_ms: Option<u64>,
    end_ms: Option<u64>,
}

impl Unit {
    fn new(text: String, start_ms: Option<u64>, end_ms: Option<u64>) -> Self {
        let chars = text.chars().count();
        Self {
            text,
            chars,
            start_ms,
            end_ms,
        }
    }
}

/// The transcript parts in time order, then the note parts, which have no times.
pub fn meeting_parts(lines: &[TranscriptLine], note: Option<&str>) -> Vec<NewPart> {
    let transcript = lines.iter().map(|l| {
        let text = transcript_text(std::slice::from_ref(l));
        Unit::new(text, Some(l.start_ms), Some(l.end_ms))
    });
    let note_lines = note
        .unwrap_or("")
        .split_inclusive('\n')
        .filter(|l| !l.trim().is_empty())
        .map(|l| Unit::new(l.to_owned(), None, None));
    let mut parts = pack(transcript.collect());
    parts.extend(pack(note_lines.collect()));
    parts
}

/// Cuts a unit that is longer than a part into pieces that overlap.
fn split_long(unit: Unit) -> Vec<Unit> {
    if unit.chars <= PART_CHARS {
        return vec![unit];
    }
    let chars: Vec<char> = unit.text.chars().collect();
    let step = PART_CHARS - OVERLAP_CHARS;
    let mut pieces = Vec::new();
    let mut from = 0;
    loop {
        let to = (from + PART_CHARS).min(chars.len());
        let text: String = chars[from..to].iter().collect();
        pieces.push(Unit::new(text, unit.start_ms, unit.end_ms));
        if to == chars.len() {
            return pieces;
        }
        from += step;
    }
}

/// Packs units into parts of at most `PART_CHARS`. Each new part starts with the
/// last units of the part before it, up to `OVERLAP_CHARS`.
fn pack(units: Vec<Unit>) -> Vec<NewPart> {
    let mut parts = Vec::new();
    let mut current: Vec<Unit> = Vec::new();
    for unit in units.into_iter().flat_map(split_long) {
        let used: usize = current.iter().map(|u| u.chars).sum();
        if !current.is_empty() && used + unit.chars > PART_CHARS {
            parts.push(to_part(&current));
            current = overlap_tail(current);
            let kept: usize = current.iter().map(|u| u.chars).sum();
            if kept + unit.chars > PART_CHARS {
                current.clear();
            }
        }
        current.push(unit);
    }
    if !current.is_empty() {
        parts.push(to_part(&current));
    }
    parts
}

/// The last units of a part that fit in `OVERLAP_CHARS`, never all of them.
fn overlap_tail(mut units: Vec<Unit>) -> Vec<Unit> {
    let mut chars = 0;
    let mut keep = 0;
    for unit in units.iter().rev().take(units.len() - 1) {
        if chars + unit.chars > OVERLAP_CHARS {
            break;
        }
        chars += unit.chars;
        keep += 1;
    }
    units.split_off(units.len() - keep)
}

fn to_part(units: &[Unit]) -> NewPart {
    NewPart {
        start_ms: units.first().and_then(|u| u.start_ms),
        end_ms: units.last().and_then(|u| u.end_ms),
        text: units.iter().map(|u| u.text.as_str()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Speaker;

    fn line(start_ms: u64, text: &str) -> TranscriptLine {
        TranscriptLine {
            meeting_id: 1,
            speaker: Speaker::Others,
            start_ms,
            end_ms: start_ms + 4_000,
            text: text.into(),
        }
    }

    fn many_lines(count: usize) -> Vec<TranscriptLine> {
        (0..count)
            .map(|i| {
                line(
                    i as u64 * 5_000,
                    &format!("line {i:03} about the launch budget and the plan"),
                )
            })
            .collect()
    }

    /// The text that `next` repeats from the end of `prev`, in characters.
    fn overlap(prev: &str, next: &str) -> usize {
        (1..=prev.len().min(next.len()))
            .rev()
            .find(|&n| {
                prev.is_char_boundary(prev.len() - n)
                    && next.is_char_boundary(n)
                    && prev.ends_with(&next[..n])
            })
            .unwrap_or(0)
    }

    #[test]
    fn parts_stay_under_400_tokens_and_overlap_by_about_50() {
        let parts = meeting_parts(&many_lines(120), None);
        assert!(parts.len() > 3, "{}", parts.len());
        for p in &parts {
            assert!(p.text.chars().count() <= PART_CHARS, "{}", p.text.len());
        }
        for pair in parts.windows(2) {
            let shared = overlap(&pair[0].text, &pair[1].text);
            assert!(shared > 0 && shared <= OVERLAP_CHARS, "{shared}");
        }
    }

    #[test]
    fn a_line_longer_than_a_part_is_cut_inside() {
        let long = "salita ".repeat(600);
        let parts = meeting_parts(&[line(0, &long)], None);
        assert!(parts.len() >= 3, "{}", parts.len());
        for p in &parts {
            assert!(p.text.chars().count() <= PART_CHARS);
            assert_eq!((p.start_ms, p.end_ms), (Some(0), Some(4_000)));
        }
    }

    #[test]
    fn note_parts_follow_the_transcript_parts_without_times() {
        let parts = meeting_parts(
            &many_lines(3),
            Some("## Summary\nThe launch moves to March 29."),
        );
        let last = parts.last().unwrap();
        assert!(last.text.contains("March 29"));
        assert_eq!((last.start_ms, last.end_ms), (None, None));
        assert!(parts[0].start_ms.is_some());
    }

    #[test]
    fn a_meeting_with_only_notes_still_has_parts() {
        let parts = meeting_parts(&[], Some("- budget ok"));
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].text, "- budget ok");
    }

    #[test]
    fn parts_keep_the_start_and_end_time_of_their_lines() {
        let lines = many_lines(120);
        let parts = meeting_parts(&lines, None);
        assert_eq!(parts[0].start_ms, Some(0));
        assert_eq!(
            parts.last().unwrap().end_ms,
            Some(lines.last().unwrap().end_ms)
        );
        assert!(parts[1].start_ms.unwrap() > 0);
    }
}
