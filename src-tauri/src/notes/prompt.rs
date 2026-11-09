//! The messages that go to the notes model.

use crate::db::templates::Template;
use crate::llm::ChatMessage;
use crate::session::clock;
use crate::types::TranscriptLine;

/// Above this estimate, the transcript is summarized in parts first.
pub const LONG_TRANSCRIPT_TOKENS: usize = 100_000;
/// The largest part of a long transcript.
pub const PART_TOKENS: usize = 80_000;

/// Characters ÷ 4.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count() / 4
}

/// One line for each transcript line: `[m:ss] You: text`.
pub fn transcript_text(lines: &[TranscriptLine]) -> String {
    lines
        .iter()
        .map(|l| {
            format!(
                "[{}] {}: {}\n",
                clock(l.start_ms),
                l.speaker.label(),
                l.text
            )
        })
        .collect()
}

/// Splits the lines into parts of at most `max_tokens`, at line ends.
pub fn split_lines(lines: &[TranscriptLine], max_tokens: usize) -> Vec<Vec<TranscriptLine>> {
    let mut parts: Vec<Vec<TranscriptLine>> = Vec::new();
    let mut current: Vec<TranscriptLine> = Vec::new();
    let mut chars = 0usize;
    for line in lines {
        let len = transcript_text(std::slice::from_ref(line)).chars().count();
        if !current.is_empty() && (chars + len) / 4 > max_tokens {
            parts.push(std::mem::take(&mut current));
            chars = 0;
        }
        chars += len;
        current.push(line.clone());
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

fn system_prompt(template: &Template) -> String {
    let headings: String = template
        .sections
        .iter()
        .map(|s| format!("## {}\n", s.name))
        .collect();
    let guide: String = template
        .sections
        .iter()
        .filter(|s| !s.hint.is_empty())
        .map(|s| format!("- {}: {}\n", s.name, s.hint))
        .collect();
    let guide = if guide.is_empty() {
        String::new()
    } else {
        format!("What each section holds:\n{guide}")
    };
    format!(
        "You write meeting notes in Markdown.\n\
         Use only facts from the rough notes and the transcript. Do not invent names, numbers or dates.\n\
         Write these headings, one time each, in this order:\n{headings}{guide}\
         Copy each line of the rough notes word for word into the section where it fits. \
         Do not change, merge or drop those lines. Add the details from the transcript below them.\n\
         Write each action item as `- [ ] Owner: task`. If the owner is not known, write `- [ ] Unknown: task`.\n\
         If a section has no content, write `- None.`\n\
         \"You\" is the user. \"Others\" is everyone else in the call.\n\
         Template instructions: {}",
        template.instructions
    )
}

fn rough_notes(user_notes: &str) -> &str {
    if user_notes.trim().is_empty() {
        "(no rough notes)"
    } else {
        user_notes.trim()
    }
}

pub fn build_messages(
    template: &Template,
    user_notes: &str,
    lines: &[TranscriptLine],
) -> Vec<ChatMessage> {
    let transcript = if lines.is_empty() {
        "(no transcript)".to_owned()
    } else {
        transcript_text(lines)
    };
    vec![
        ChatMessage::system(system_prompt(template)),
        ChatMessage::user(format!(
            "Rough notes:\n{}\n\nTranscript:\n{transcript}",
            rough_notes(user_notes)
        )),
    ]
}

pub fn summary_messages(part: &str, index: usize, count: usize) -> Vec<ChatMessage> {
    vec![
        ChatMessage::system(
            "Summarize part of a meeting transcript. Keep every decision, number, date, name and task, \
             with who said it. Use short bullet points.",
        ),
        ChatMessage::user(format!("Summarize part {index} of {count}:\n{part}")),
    ]
}

pub fn build_messages_from_summaries(
    template: &Template,
    user_notes: &str,
    summaries: &[String],
) -> Vec<ChatMessage> {
    let joined: String = summaries
        .iter()
        .enumerate()
        .map(|(i, s)| format!("Part {} of {}:\n{s}\n\n", i + 1, summaries.len()))
        .collect();
    vec![
        ChatMessage::system(system_prompt(template)),
        ChatMessage::user(format!(
            "Rough notes:\n{}\n\nThe transcript is long, so here are summaries of its parts, in order:\n{joined}",
            rough_notes(user_notes)
        )),
    ]
}

pub fn title_messages(note_md: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage::system(
            "Write a title of at most 6 words for these meeting notes. Write only the title.",
        ),
        ChatMessage::user(format!("Write a title for:\n{note_md}")),
    ]
}

/// The first line, without quotes and Markdown marks, at most 6 words.
pub fn clean_title(raw: &str) -> String {
    let first = raw.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let trimmed = first
        .trim()
        .trim_matches(|c| matches!(c, '"' | '\'' | '#' | '*' | '.'))
        .trim();
    trimmed
        .split_whitespace()
        .take(6)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::templates::Section;
    use crate::types::Speaker;

    fn template() -> Template {
        Template {
            id: 1,
            name: "General".into(),
            description: String::new(),
            sections: ["Summary", "Key points", "Action items"]
                .into_iter()
                .map(|name| Section {
                    name: name.into(),
                    hint: String::new(),
                })
                .collect(),
            instructions: "Be short.".into(),
            built_in: true,
            uses: 0,
        }
    }

    fn line(speaker: Speaker, start_ms: u64, text: &str) -> TranscriptLine {
        TranscriptLine {
            meeting_id: 1,
            speaker,
            start_ms,
            end_ms: start_ms + 1000,
            text: text.into(),
        }
    }

    #[test]
    fn prompt_has_one_heading_per_section() {
        let m = build_messages(&template(), "budget ok", &[]);
        let system = &m[0].content;
        for section in ["## Summary", "## Key points", "## Action items"] {
            assert_eq!(system.matches(section).count(), 1, "{section}");
        }
        assert!(system.contains("- [ ] Owner: task"));
        assert!(system.contains("word for word"));
        assert!(system.contains("Be short."));
    }

    #[test]
    fn prompt_marks_speakers_and_times() {
        let lines = [
            line(Speaker::You, 5_000, "hello"),
            line(Speaker::Others, 62_000, "hi"),
        ];
        let user = &build_messages(&template(), "budget ok", &lines)[1].content;
        assert!(user.contains("[0:05] You: hello\n[1:02] Others: hi"));
        assert!(user.contains("budget ok"));
    }

    #[test]
    fn empty_transcript_is_marked() {
        let user = &build_messages(&template(), "budget ok", &[])[1].content;
        assert!(user.contains("(no transcript)"));
    }

    #[test]
    fn estimate_tokens_is_chars_divided_by_four() {
        assert_eq!(estimate_tokens("abcdefgh"), 2);
    }

    #[test]
    fn split_keeps_parts_under_the_limit_at_line_ends() {
        let lines: Vec<TranscriptLine> = (0..10)
            .map(|i| line(Speaker::You, i * 1000, &"x".repeat(400)))
            .collect();
        let parts = split_lines(&lines, 300);
        assert_eq!(parts.len(), 5);
        assert_eq!(parts.iter().map(Vec::len).sum::<usize>(), 10);
        assert!(
            parts
                .iter()
                .all(|p| estimate_tokens(&transcript_text(p)) <= 300)
        );
    }

    #[test]
    fn section_hints_show_in_the_system_prompt() {
        let mut t = template();
        t.sections[1].hint = "The main facts and ideas".into();
        let system = &build_messages(&t, "", &[])[0].content;
        assert!(system.contains("## Key points\n"));
        assert!(system.contains("- Key points: The main facts and ideas"));
        assert!(!system.contains("- Summary:"));
    }
}
