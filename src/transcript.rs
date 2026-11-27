use crate::{BlockKind, Desk, ReplyBlock, ThemeColors, Transcript};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use slint::{Color, ComponentHandle, ModelRc, StyledText, VecModel};
use std::{fmt::Write, sync::LazyLock};
#[path = "transcript_media.rs"]
mod media;
use syntect::{
    easy::ScopeRegionIterator,
    parsing::{ParseState, Scope, ScopeStack, SyntaxSet},
};

pub fn connect(ui: &Desk) {
    let transcript = ui.global::<Transcript>();
    media::connect(ui);
    transcript.on_formatted(|text| formatted(&text));
    transcript.on_blocks(|text| ModelRc::new(VecModel::from(blocks(&text))));
    transcript.on_highlight(|source, language, colors| {
        highlight(&source, &language, &colors)
            .unwrap_or_else(|| StyledText::from_plain_text(&source))
    });
}

fn formatted(text: &str) -> bool {
    text.lines().any(|line| bare_media(line).is_some())
        || Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH).any(
            |event| {
                matches!(event, Event::Start(ref tag) if !matches!(tag, Tag::Paragraph))
                    || matches!(event, Event::Code(_) | Event::InlineHtml(_) | Event::Rule)
            },
        )
}

fn bare_media(line: &str) -> Option<ReplyBlock> {
    let target = line.trim().trim_matches('`').trim_matches('"');
    if !(std::path::Path::new(target).is_absolute()
        || target.starts_with("./")
        || target.starts_with("https://")
        || target.starts_with("http://")
        || target.starts_with("file:///"))
    {
        return None;
    }
    let (name, kind) = media::attachment(target)?;
    if !matches!(kind.as_str(), "Image" | "Video") {
        return None;
    }
    Some(ReplyBlock {
        kind: if kind == "Image" {
            BlockKind::Image
        } else {
            BlockKind::Video
        },
        text: name.into(),
        target: target.into(),
        remote: target.starts_with("http://") || target.starts_with("https://"),
        ..Default::default()
    })
}

fn prose(text: &str, blocks: &mut Vec<ReplyBlock>) {
    let mut remaining = text;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if let Some(media) = bare_media(line) {
            markdown_prose(&remaining[..offset], blocks);
            blocks.push(media);
            remaining = &remaining[offset + line.len()..];
            offset = 0;
        } else {
            offset += line.len();
        }
    }
    markdown_prose(remaining, blocks);
}

fn markdown_prose(text: &str, blocks: &mut Vec<ReplyBlock>) {
    let text = text.trim_matches(['\r', '\n']);
    if text.is_empty() {
        return;
    }
    if !formatted(text) {
        blocks.push(ReplyBlock {
            kind: BlockKind::Prose,
            text: text.into(),
            ..Default::default()
        });
        return;
    }
    let rich = StyledText::from_markdown(text).ok();
    let mut readable = String::new();
    let mut attachments = Vec::new();
    for event in Parser::new_ext(text, Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Text(s) | Event::Code(s) | Event::InlineHtml(s) | Event::Html(s) => {
                readable.push_str(&s)
            }
            Event::SoftBreak | Event::HardBreak | Event::End(TagEnd::Paragraph | TagEnd::Item) => {
                readable.push('\n')
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                if let Some((name, kind)) = media::attachment(&dest_url) {
                    attachments.push(ReplyBlock {
                        kind: if kind == "Image" {
                            BlockKind::Image
                        } else if kind == "Video" {
                            BlockKind::Video
                        } else {
                            BlockKind::Attachment
                        },
                        remote: dest_url.starts_with("https://") || dest_url.starts_with("http://"),
                        text: name.into(),
                        language: kind.into(),
                        target: dest_url.as_ref().into(),
                        ..Default::default()
                    });
                } else if rich.is_none() {
                    attachments.push(ReplyBlock {
                        kind: BlockKind::Attachment,
                        text: dest_url.as_ref().into(),
                        language: "Link".into(),
                        target: dest_url.as_ref().into(),
                        ..Default::default()
                    });
                }
            }
            _ => {}
        }
    }
    blocks.push(ReplyBlock {
        kind: if rich.is_some() {
            BlockKind::Rich
        } else {
            BlockKind::Prose
        },
        text: readable.trim_end_matches('\n').into(),
        styled: rich.unwrap_or_default(),
        ..Default::default()
    });
    blocks.extend(attachments);
}

fn blocks(source: &str) -> Vec<ReplyBlock> {
    let mut blocks = Vec::new();
    let mut consumed = 0;
    let mut events = Parser::new_ext(
        source,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH,
    )
    .into_offset_iter();
    while let Some((event, range)) = events.next() {
        if let Event::Start(Tag::Table(_)) = event {
            prose(&source[consumed..range.start], &mut blocks);
            let mut cells = Vec::new();
            let mut header = false;
            for (event, span) in events.by_ref() {
                match event {
                    Event::Start(Tag::TableHead) => header = true,
                    Event::Start(Tag::TableCell) => {
                        let cell = source[span].trim();
                        cells.push(
                            StyledText::from_markdown(cell)
                                .unwrap_or_else(|_| StyledText::from_plain_text(cell)),
                        );
                    }
                    Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                        blocks.push(ReplyBlock {
                            kind: BlockKind::TableRow,
                            header,
                            cells: ModelRc::new(VecModel::from(std::mem::take(&mut cells))),
                            ..Default::default()
                        });
                        header = false;
                    }
                    Event::End(TagEnd::Table) => {
                        consumed = span.end;
                        break;
                    }
                    _ => {}
                }
            }
            continue;
        }
        let mut target = String::new();
        let (kind, language, end) = match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                target = dest_url.into_string();
                (
                    if media::attachment(&target).is_some_and(|(_, k)| k == "Video") {
                        BlockKind::Video
                    } else {
                        BlockKind::Image
                    },
                    String::new(),
                    TagEnd::Image,
                )
            }
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
            remote: target.starts_with("https://") || target.starts_with("http://"),
            target: target.into(),
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
    fn links_images_and_tables_survive_mixed_markdown() {
        use slint::Model;
        let input = "**Ready** [Get the project](<C:/My Files/Game.zip>)\n\n![Lobby](build/Lobby.png)\n\n| Check | Result |\n| --- | --- |\n| Types | **Passed** |";
        let parsed = blocks(input);
        assert_eq!(
            parsed.iter().map(|b| b.kind).collect::<Vec<_>>(),
            [
                BlockKind::Rich,
                BlockKind::Attachment,
                BlockKind::Image,
                BlockKind::TableRow,
                BlockKind::TableRow
            ]
        );
        assert_eq!(parsed[1].text.as_str(), "Game.zip");
        assert_eq!(parsed[1].target.as_str(), "C:/My Files/Game.zip");
        assert_eq!(parsed[2].target.as_str(), "build/Lobby.png");
        assert_eq!(parsed[2].text.as_str(), "Lobby");
        assert!(parsed[3].header);
        assert_eq!(parsed[4].cells.row_count(), 2);
        assert_eq!(
            parsed[4].cells.row_data(0).unwrap(),
            StyledText::from_plain_text("Types")
        );
        assert_eq!(blocks("```md\n![no embed](a.png)\n```").len(), 1);
    }

    #[test]
    fn media_links_paths_and_fenced_examples_are_distinct() {
        let reply = "[Play](<videos/First clip.mp4>)\n\n![Clip](https://example.com/a.webm)\n\n./capture.png\n\n~~~text\n./other.png\n~~~";
        let parsed = blocks(reply);
        let embeds: Vec<_> = parsed
            .iter()
            .filter(|b| matches!(b.kind, BlockKind::Video | BlockKind::Image))
            .collect();
        assert_eq!(embeds.len(), 3);
        assert_eq!(embeds[0].kind, BlockKind::Video);
        assert_eq!(embeds[1].kind, BlockKind::Video);
        assert!(embeds[1].remote);
        assert_eq!(embeds[2].target.as_str(), "./capture.png");
        assert_eq!(parsed.last().unwrap().kind, BlockKind::Code);
    }

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
        assert_eq!(blocks("~~~py\nÏ€ = 3")[0].text.as_str(), "Ï€ = 3");
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
