use crate::{BlockKind, Desk, ReplyBlock, ThemeColors, Transcript};
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use slint::{Color, ComponentHandle, ModelRc, StyledText, VecModel};
use std::{fmt::Write, sync::LazyLock};
use syntect::{
    easy::ScopeRegionIterator,
    parsing::{ParseState, Scope, ScopeStack, SyntaxSet},
};

pub fn connect(ui: &Desk) {
    let transcript = ui.global::<Transcript>();
    transcript.on_blocks(|text| ModelRc::new(VecModel::from(blocks(&text))));
    transcript.on_highlight(|source, language, colors| {
        highlight(&source, &language, &colors)
            .unwrap_or_else(|| StyledText::from_plain_text(&source))
    });
}

fn prose(text: &str, blocks: &mut Vec<ReplyBlock>) {
    let text = text.trim_matches(['\r', '\n']);
    if text.is_empty() {
        return;
    }
    // Keep literal links, paths and HTML selectable, including their original destinations.
    let rich = if (text.contains("**") || text.contains('`')) && !text.contains(['[', '<']) {
        StyledText::from_markdown(text).ok()
    } else {
        None
    };
    blocks.push(ReplyBlock {
        kind: if rich.is_some() {
            BlockKind::Rich
        } else {
            BlockKind::Prose
        },
        text: text.into(),
        styled: rich.unwrap_or_default(),
        ..Default::default()
    });
}

fn blocks(source: &str) -> Vec<ReplyBlock> {
    let mut blocks = Vec::new();
    let mut consumed = 0;
    let mut events = Parser::new(source).into_offset_iter();
    while let Some((event, range)) = events.next() {
        let (kind, language, end) = match event {
            Event::Start(Tag::CodeBlock(info)) => {
                let name = match info {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_owned()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                (BlockKind::Code, name, TagEnd::CodeBlock)
            }
            Event::Start(Tag::Heading { level, .. }) => {
                (BlockKind::Heading, String::new(), TagEnd::Heading(level))
            }
            _ => continue,
        };
        prose(&source[consumed..range.start], &mut blocks);
        let mut text = String::new();
        consumed = range.end;
        for (event, range) in events.by_ref() {
            match event {
                Event::End(tag) if tag == end => {
                    consumed = range.end;
                    break;
                }
                Event::Text(word) | Event::Code(word) | Event::InlineHtml(word) => {
                    text.push_str(&word)
                }
                Event::HardBreak | Event::SoftBreak => text.push('\n'),
                _ => {}
            }
        }
        blocks.push(ReplyBlock {
            kind,
            language: language.into(),
            text: text.into(),
            ..Default::default()
        });
    }
    prose(&source[consumed..], &mut blocks);
    blocks
}

#[derive(Clone, Copy)]
enum Ink {
    Keyword,
    String,
    Value,
    Symbol,
    Comment,
    Added,
    Removed,
}

impl Ink {
    fn color(self, colors: &ThemeColors) -> Color {
        match self {
            Self::Keyword => colors.keyword,
            Self::String => colors.accent,
            Self::Value => colors.halfway,
            Self::Symbol => colors.symbol,
            Self::Comment => colors.muted,
            Self::Added => colors.ample,
            Self::Removed => colors.scarce,
        }
    }
}

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static SCOPES: LazyLock<Vec<(Scope, Ink)>> = LazyLock::new(|| {
    [
        ("comment", Ink::Comment),
        ("string", Ink::String),
        ("keyword", Ink::Keyword),
        ("storage", Ink::Keyword),
        ("constant", Ink::Value),
        ("entity.name", Ink::Symbol),
        ("support", Ink::Symbol),
        ("variable.function", Ink::Symbol),
        ("markup.inserted", Ink::Added),
        ("markup.deleted", Ink::Removed),
    ]
    .into_iter()
    .map(|(scope, ink)| (scope.parse().expect("valid syntax scope"), ink))
    .collect()
});

fn highlight(source: &str, language: &str, colors: &ThemeColors) -> Option<StyledText> {
    // Bound grammar work on the UI thread. Larger snippets retain the same selectable code surface.
    if source.is_empty()
        || language.is_empty()
        || source.len() > 24 * 1024
        || source.lines().any(|line| line.len() > 2000)
    {
        return None;
    }
    let token = language.to_ascii_lowercase();
    let syntax = SYNTAXES
        .find_syntax_by_token(&token)
        .or_else(|| match token.as_str() {
            "js" | "jsx" | "ts" | "tsx" | "typescript" => SYNTAXES.find_syntax_by_extension("js"),
            "shell" | "console" | "bash" => SYNTAXES.find_syntax_by_extension("sh"),
            _ => None,
        })?;
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut markup = String::new();
    for line in source.split_inclusive('\n') {
        let ops = state.parse_line(line, &SYNTAXES).ok()?;
        for (part, op) in ScopeRegionIterator::new(&ops, line) {
            stack.apply(op).ok()?;
            if part.is_empty() {
                continue;
            }
            let ink = stack.as_slice().iter().rev().find_map(|&scope| {
                SCOPES
                    .iter()
                    .find(|(prefix, _)| prefix.is_prefix_of(scope))
                    .map(|(_, ink)| ink.color(colors))
            });
            if let Some(color) = ink {
                write!(
                    markup,
                    "<font color=\"#{:02x}{:02x}{:02x}\">",
                    color.red(),
                    color.green(),
                    color.blue()
                )
                .ok()?;
            }
            // Slint accepts color spans through Markdown. Encode literal punctuation
            // and whitespace so source code can never introduce markup or lose indentation.
            for ch in part.chars() {
                if ch.is_ascii_punctuation() || ch.is_ascii_whitespace() {
                    write!(markup, "&#{};", u32::from(ch)).ok()?;
                } else {
                    markup.push(ch);
                }
            }
            if ink.is_some() {
                markup.push_str("</font>");
            }
        }
    }
    StyledText::from_markdown(&markup).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fenced_and_streaming_code_preserve_contents_and_order() {
        let text = "# Result\n\nBefore.\n\n````rust\n  let s = \"```\";\n\n````\n\nAfter.";
        let parsed = blocks(text);
        assert_eq!(
            parsed.iter().map(|b| b.kind).collect::<Vec<_>>(),
            [
                BlockKind::Heading,
                BlockKind::Prose,
                BlockKind::Code,
                BlockKind::Prose
            ]
        );
        assert_eq!(parsed[2].text.as_str(), "  let s = \"```\";\n\n");
        assert_eq!(blocks("~~~py\nπ = 3")[0].text.as_str(), "π = 3");
        assert!(
            highlight(
                "<script>\n    fn test() {}\n",
                "rust",
                &ThemeColors::default()
            )
            .is_some()
        );
    }
}
