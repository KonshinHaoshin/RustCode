use super::theme::{TerminalTheme, GUTTER};
use lru::LruCache;
use pulldown_cmark::{Alignment, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    num::NonZeroUsize,
    sync::{Mutex, OnceLock},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const MARKDOWN_CACHE_MAX: usize = 256;
const TABLE_VERTICAL_THRESHOLD: usize = 4;
const TABLE_SAFETY_MARGIN: usize = 4;
const MIN_TABLE_COL_WIDTH: usize = 3;

fn markdown_cache() -> &'static Mutex<LruCache<u64, Vec<MarkdownBlock>>> {
    static CACHE: OnceLock<Mutex<LruCache<u64, Vec<MarkdownBlock>>>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(LruCache::new(
            NonZeroUsize::new(MARKDOWN_CACHE_MAX).expect("cache size is non-zero"),
        ))
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatRenderLine {
    pub plain_text: String,
    pub spans: Vec<(String, Style)>,
}

impl ChatRenderLine {
    pub fn plain(text: String, style: Style) -> Self {
        Self {
            plain_text: text.clone(),
            spans: vec![(text, style)],
        }
    }

    pub fn empty() -> Self {
        Self {
            plain_text: String::new(),
            spans: vec![(String::new(), Style::default())],
        }
    }

    pub fn from_spans(spans: Vec<(String, Style)>) -> Self {
        let plain_text = spans.iter().map(|(text, _)| text.as_str()).collect();
        Self { plain_text, spans }
    }

    pub fn to_line(
        &self,
        selection: Option<(usize, usize)>,
        theme: TerminalTheme,
    ) -> Line<'static> {
        if let Some((start, end)) = selection {
            return Line::from(apply_selection(&self.spans, start, end, theme));
        }

        Line::from(
            self.spans
                .iter()
                .map(|(text, style)| Span::styled(text.clone(), *style))
                .collect::<Vec<_>>(),
        )
    }
}

#[derive(Clone, Debug)]
enum MarkdownBlock {
    Paragraph(Vec<InlineChunk>),
    Heading(u8, Vec<InlineChunk>),
    CodeFence(String),
    BlockQuote(Vec<MarkdownBlock>),
    List {
        ordered: bool,
        start: usize,
        items: Vec<Vec<MarkdownBlock>>,
    },
    Table {
        alignments: Vec<Alignment>,
        headers: Vec<Vec<InlineChunk>>,
        rows: Vec<Vec<Vec<InlineChunk>>>,
    },
    ThematicBreak,
}

#[derive(Clone, Debug)]
struct InlineChunk {
    text: String,
    style: InlineStyle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct InlineStyle {
    strong: bool,
    emphasis: bool,
    inline_code: bool,
    link: bool,
}

pub fn render_assistant_markdown(
    content: &str,
    theme: TerminalTheme,
    width: u16,
) -> Vec<ChatRenderLine> {
    let stripped = strip_prompt_xml_tags(content);
    if stripped.trim().is_empty() {
        return vec![ChatRenderLine::plain(
            format!("{} ", GUTTER),
            Style::default().fg(theme.subtle),
        )];
    }

    let blocks = parse_markdown_cached(&stripped);
    render_blocks(&blocks, theme, width.max(1) as usize)
}

pub fn wrap_plain_line(text: &str, width: usize) -> Vec<String> {
    wrap_segments(&[(text.to_string(), Style::default())], width)
        .into_iter()
        .map(|line| line.plain_text)
        .collect()
}

fn apply_selection(
    spans: &[(String, Style)],
    selection_start: usize,
    selection_end: usize,
    theme: TerminalTheme,
) -> Vec<Span<'static>> {
    let selected_style = Style::default()
        .bg(theme.shimmer)
        .fg(Color::Black)
        .add_modifier(Modifier::BOLD);
    let mut result = Vec::new();
    let mut offset = 0usize;

    for (text, style) in spans {
        let chars = text.chars().collect::<Vec<_>>();
        if chars.is_empty() {
            continue;
        }
        let span_start = offset;
        let span_end = offset + chars.len().saturating_sub(1);

        if span_end < selection_start || span_start > selection_end {
            result.push(Span::styled(text.clone(), *style));
            offset += chars.len();
            continue;
        }

        let local_start = selection_start.saturating_sub(span_start).min(chars.len());
        let local_end = selection_end
            .saturating_sub(span_start)
            .min(chars.len().saturating_sub(1));

        if local_start > 0 {
            result.push(Span::styled(
                chars[..local_start].iter().collect::<String>(),
                *style,
            ));
        }
        if local_start <= local_end {
            result.push(Span::styled(
                chars[local_start..=local_end].iter().collect::<String>(),
                selected_style,
            ));
        }
        if local_end + 1 < chars.len() {
            result.push(Span::styled(
                chars[local_end + 1..].iter().collect::<String>(),
                *style,
            ));
        }

        offset += chars.len();
    }

    if result.is_empty() {
        result.push(Span::raw(String::new()));
    }

    result
}

fn strip_prompt_xml_tags(content: &str) -> String {
    let mut stripped = String::with_capacity(content.len());
    let mut in_tag = false;
    for ch in content.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => stripped.push(ch),
            _ => {}
        }
    }
    stripped
}

fn has_markdown_syntax(content: &str) -> bool {
    let sample = if content.len() > 500 {
        &content[..500]
    } else {
        content
    };
    sample.contains('\n')
        || sample.contains('#')
        || sample.contains('*')
        || sample.contains('`')
        || sample.contains('|')
        || sample.contains('>')
        || sample.contains("- ")
        || sample.contains("1. ")
}

fn parse_markdown_cached(content: &str) -> Vec<MarkdownBlock> {
    if !has_markdown_syntax(content) {
        return vec![MarkdownBlock::Paragraph(vec![InlineChunk {
            text: content.to_string(),
            style: InlineStyle::default(),
        }])];
    }

    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    let key = hasher.finish();

    if let Ok(mut cache) = markdown_cache().lock() {
        if let Some(hit) = cache.get(&key) {
            return hit.clone();
        }
    }

    let parsed = parse_blocks(content);

    if let Ok(mut cache) = markdown_cache().lock() {
        cache.put(key, parsed.clone());
    }

    parsed
}

fn parse_blocks(content: &str) -> Vec<MarkdownBlock> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let events = Parser::new_ext(content, options).collect::<Vec<_>>();
    let mut cursor = 0usize;
    parse_blocks_until(&events, &mut cursor, None)
}

fn parse_blocks_until(
    events: &[Event<'_>],
    cursor: &mut usize,
    end: Option<TagEnd>,
) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();

    while *cursor < events.len() {
        match &events[*cursor] {
            Event::End(tag_end) if end.as_ref() == Some(tag_end) => {
                *cursor += 1;
                break;
            }
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    *cursor += 1;
                    blocks.push(MarkdownBlock::Paragraph(parse_inlines_until(
                        events,
                        cursor,
                        TagEnd::Paragraph,
                    )));
                }
                Tag::Heading { level, .. } => {
                    let heading_level = match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    };
                    *cursor += 1;
                    blocks.push(MarkdownBlock::Heading(
                        heading_level,
                        parse_inlines_until(events, cursor, TagEnd::Heading(*level)),
                    ));
                }
                Tag::CodeBlock(_) => {
                    *cursor += 1;
                    blocks.push(MarkdownBlock::CodeFence(parse_codeblock(events, cursor)));
                }
                Tag::BlockQuote => {
                    *cursor += 1;
                    blocks.push(MarkdownBlock::BlockQuote(parse_blocks_until(
                        events,
                        cursor,
                        Some(TagEnd::BlockQuote),
                    )));
                }
                Tag::List(start) => {
                    let ordered = start.is_some();
                    let start = start.unwrap_or(1) as usize;
                    *cursor += 1;
                    blocks.push(parse_list(events, cursor, ordered, start));
                }
                Tag::Table(alignments) => {
                    let alignments = alignments.clone();
                    *cursor += 1;
                    blocks.push(parse_table(events, cursor, alignments));
                }
                _ => {
                    *cursor += 1;
                }
            },
            Event::Rule => {
                blocks.push(MarkdownBlock::ThematicBreak);
                *cursor += 1;
            }
            Event::Text(text) => {
                let text = text.trim();
                if !text.is_empty() {
                    blocks.push(MarkdownBlock::Paragraph(vec![InlineChunk {
                        text: text.to_string(),
                        style: InlineStyle::default(),
                    }]));
                }
                *cursor += 1;
            }
            _ => {
                *cursor += 1;
            }
        }
    }

    blocks
}

fn parse_codeblock(events: &[Event<'_>], cursor: &mut usize) -> String {
    let mut code = String::new();
    while *cursor < events.len() {
        match &events[*cursor] {
            Event::End(TagEnd::CodeBlock) => {
                *cursor += 1;
                break;
            }
            Event::Text(text) | Event::Code(text) | Event::Html(text) | Event::InlineHtml(text) => {
                code.push_str(text)
            }
            Event::SoftBreak | Event::HardBreak => code.push('\n'),
            _ => {}
        }
        *cursor += 1;
    }
    code
}

fn parse_list(
    events: &[Event<'_>],
    cursor: &mut usize,
    ordered: bool,
    start: usize,
) -> MarkdownBlock {
    let mut items = Vec::new();
    while *cursor < events.len() {
        match &events[*cursor] {
            Event::Start(Tag::Item) => {
                *cursor += 1;
                items.push(parse_blocks_until(events, cursor, Some(TagEnd::Item)));
            }
            Event::End(TagEnd::List(_)) => {
                *cursor += 1;
                break;
            }
            _ => {
                *cursor += 1;
            }
        }
    }
    MarkdownBlock::List {
        ordered,
        start,
        items,
    }
}

fn parse_table(
    events: &[Event<'_>],
    cursor: &mut usize,
    alignments: Vec<Alignment>,
) -> MarkdownBlock {
    let mut headers = Vec::new();
    let mut rows = Vec::new();

    while *cursor < events.len() {
        match &events[*cursor] {
            Event::Start(Tag::TableHead) => {
                *cursor += 1;
                headers = parse_table_cells(events, cursor, TagEnd::TableHead);
            }
            Event::Start(Tag::TableRow) => {
                *cursor += 1;
                rows.push(parse_table_cells(events, cursor, TagEnd::TableRow));
            }
            Event::End(TagEnd::Table) => {
                *cursor += 1;
                break;
            }
            _ => {
                *cursor += 1;
            }
        }
    }

    MarkdownBlock::Table {
        alignments,
        headers,
        rows,
    }
}

fn parse_table_cells(
    events: &[Event<'_>],
    cursor: &mut usize,
    row_end: TagEnd,
) -> Vec<Vec<InlineChunk>> {
    let mut cells = Vec::new();
    while *cursor < events.len() {
        match &events[*cursor] {
            Event::Start(Tag::TableCell) => {
                *cursor += 1;
                cells.push(parse_inlines_until(events, cursor, TagEnd::TableCell));
            }
            Event::End(tag_end) if *tag_end == row_end => {
                *cursor += 1;
                break;
            }
            _ => {
                *cursor += 1;
            }
        }
    }
    cells
}

fn parse_inlines_until(events: &[Event<'_>], cursor: &mut usize, end: TagEnd) -> Vec<InlineChunk> {
    let mut chunks = Vec::new();

    while *cursor < events.len() {
        match &events[*cursor] {
            Event::End(tag_end) if *tag_end == end => {
                *cursor += 1;
                break;
            }
            Event::Start(tag) => match tag {
                Tag::Emphasis => {
                    *cursor += 1;
                    let inner = parse_inlines_until(events, cursor, TagEnd::Emphasis);
                    push_styled_chunks(
                        &mut chunks,
                        inner,
                        InlineStyle {
                            emphasis: true,
                            ..InlineStyle::default()
                        },
                    );
                }
                Tag::Strong => {
                    *cursor += 1;
                    let inner = parse_inlines_until(events, cursor, TagEnd::Strong);
                    push_styled_chunks(
                        &mut chunks,
                        inner,
                        InlineStyle {
                            strong: true,
                            ..InlineStyle::default()
                        },
                    );
                }
                Tag::Link { .. } => {
                    *cursor += 1;
                    let inner = parse_inlines_until(events, cursor, TagEnd::Link);
                    push_styled_chunks(
                        &mut chunks,
                        inner,
                        InlineStyle {
                            link: true,
                            ..InlineStyle::default()
                        },
                    );
                }
                _ => {
                    *cursor += 1;
                }
            },
            Event::Text(text) => {
                chunks.push(InlineChunk {
                    text: text.to_string(),
                    style: InlineStyle::default(),
                });
                *cursor += 1;
            }
            Event::Code(text) => {
                chunks.push(InlineChunk {
                    text: text.to_string(),
                    style: InlineStyle {
                        inline_code: true,
                        ..InlineStyle::default()
                    },
                });
                *cursor += 1;
            }
            Event::SoftBreak => {
                chunks.push(InlineChunk {
                    text: " ".to_string(),
                    style: InlineStyle::default(),
                });
                *cursor += 1;
            }
            Event::HardBreak => {
                chunks.push(InlineChunk {
                    text: "\n".to_string(),
                    style: InlineStyle::default(),
                });
                *cursor += 1;
            }
            Event::InlineHtml(text) | Event::Html(text) | Event::FootnoteReference(text) => {
                chunks.push(InlineChunk {
                    text: text.to_string(),
                    style: InlineStyle::default(),
                });
                *cursor += 1;
            }
            _ => {
                *cursor += 1;
            }
        }
    }

    chunks
}

fn push_styled_chunks(target: &mut Vec<InlineChunk>, inner: Vec<InlineChunk>, patch: InlineStyle) {
    for mut chunk in inner {
        chunk.style.strong |= patch.strong;
        chunk.style.emphasis |= patch.emphasis;
        chunk.style.inline_code |= patch.inline_code;
        chunk.style.link |= patch.link;
        target.push(chunk);
    }
}

fn render_blocks(
    blocks: &[MarkdownBlock],
    theme: TerminalTheme,
    width: usize,
) -> Vec<ChatRenderLine> {
    let mut lines = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        render_block(block, theme, width, 0, &mut lines);
        if index + 1 < blocks.len() {
            lines.push(ChatRenderLine::empty());
        }
    }
    if lines.is_empty() {
        lines.push(ChatRenderLine::plain(
            format!("{} ", GUTTER),
            Style::default().fg(theme.subtle),
        ));
    }
    lines
}

fn render_block(
    block: &MarkdownBlock,
    theme: TerminalTheme,
    width: usize,
    indent: usize,
    lines: &mut Vec<ChatRenderLine>,
) {
    match block {
        MarkdownBlock::Paragraph(chunks) => render_wrapped_inline_block(
            lines,
            vec![
                (
                    format!("{}{}", " ".repeat(indent), GUTTER),
                    Style::default().fg(theme.subtle),
                ),
                (" ".to_string(), Style::default().fg(theme.text)),
            ],
            chunks,
            theme,
            width,
            Style::default().fg(theme.text),
        ),
        MarkdownBlock::Heading(level, chunks) => {
            let mut style = Style::default()
                .fg(theme.brand)
                .add_modifier(Modifier::BOLD);
            if *level == 1 {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            render_wrapped_inline_block(
                lines,
                vec![
                    (
                        format!("{}{}", " ".repeat(indent), GUTTER),
                        Style::default().fg(theme.brand),
                    ),
                    (" ".to_string(), style),
                ],
                chunks,
                theme,
                width,
                style,
            );
        }
        MarkdownBlock::CodeFence(code) => {
            let base = Style::default().fg(theme.text).bg(theme.user_msg_bg);
            let prefix = format!("{}{}", " ".repeat(indent), GUTTER);
            if code.is_empty() {
                lines.push(ChatRenderLine::plain(format!("{} ", prefix), base));
                return;
            }
            for raw_line in code.lines() {
                let text = raw_line.replace('\t', "    ");
                let segments = vec![
                    (prefix.clone(), Style::default().fg(theme.subtle)),
                    (" ".to_string(), base),
                    (text, base),
                ];
                lines.extend(wrap_segments(&segments, width));
            }
        }
        MarkdownBlock::BlockQuote(inner) => {
            for block in inner {
                render_blockquote_block(block, theme, width, indent, lines);
            }
        }
        MarkdownBlock::List {
            ordered,
            start,
            items,
        } => {
            for (index, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}.", start + index)
                } else {
                    "-".to_string()
                };
                render_list_item(item, &marker, theme, width, indent, lines);
            }
        }
        MarkdownBlock::Table {
            alignments,
            headers,
            rows,
        } => lines.extend(render_table(
            alignments, headers, rows, theme, width, indent,
        )),
        MarkdownBlock::ThematicBreak => {
            let rule_width = width
                .saturating_sub(indent + GUTTER.chars().count() + 1)
                .max(3);
            lines.push(ChatRenderLine::plain(
                format!(
                    "{}{} {}",
                    " ".repeat(indent),
                    GUTTER,
                    "─".repeat(rule_width)
                ),
                Style::default().fg(theme.subtle),
            ));
        }
    }
}

fn render_blockquote_block(
    block: &MarkdownBlock,
    theme: TerminalTheme,
    width: usize,
    indent: usize,
    lines: &mut Vec<ChatRenderLine>,
) {
    let mut quoted = Vec::new();
    render_block(block, theme, width, indent + 2, &mut quoted);
    for line in quoted {
        let mut spans = vec![
            (
                format!("{}{}", " ".repeat(indent), GUTTER),
                Style::default().fg(theme.subtle),
            ),
            (" ".to_string(), Style::default().fg(theme.muted)),
            ("│ ".to_string(), Style::default().fg(theme.muted)),
        ];
        spans.extend(line.spans);
        lines.push(ChatRenderLine::from_spans(spans));
    }
}

fn render_list_item(
    blocks: &[MarkdownBlock],
    marker: &str,
    theme: TerminalTheme,
    width: usize,
    indent: usize,
    lines: &mut Vec<ChatRenderLine>,
) {
    let base_indent = format!("{}{}", " ".repeat(indent), GUTTER);
    let continuation = format!(
        "{}{}",
        " ".repeat(indent + marker.chars().count() + 3),
        GUTTER
    );

    for (block_index, block) in blocks.iter().enumerate() {
        match block {
            MarkdownBlock::Paragraph(chunks) => {
                let prefix = if block_index == 0 {
                    vec![
                        (base_indent.clone(), Style::default().fg(theme.subtle)),
                        (" ".to_string(), Style::default().fg(theme.text)),
                        (
                            format!("{} ", marker),
                            Style::default()
                                .fg(theme.brand)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]
                } else {
                    vec![
                        (continuation.clone(), Style::default().fg(theme.subtle)),
                        (" ".to_string(), Style::default().fg(theme.text)),
                    ]
                };
                render_wrapped_inline_block(
                    lines,
                    prefix,
                    chunks,
                    theme,
                    width,
                    Style::default().fg(theme.text),
                );
            }
            other => render_block(other, theme, width, indent + 2, lines),
        }
    }
}

fn render_wrapped_inline_block(
    lines: &mut Vec<ChatRenderLine>,
    prefix: Vec<(String, Style)>,
    chunks: &[InlineChunk],
    theme: TerminalTheme,
    width: usize,
    fallback_style: Style,
) {
    let inline_segments = inline_chunks_to_segments(chunks, theme, fallback_style);
    let mut segments = prefix;
    segments.extend(inline_segments);
    lines.extend(wrap_segments(&segments, width));
}

fn inline_chunks_to_segments(
    chunks: &[InlineChunk],
    theme: TerminalTheme,
    fallback_style: Style,
) -> Vec<(String, Style)> {
    chunks
        .iter()
        .filter(|chunk| !chunk.text.is_empty())
        .map(|chunk| {
            (
                chunk.text.clone(),
                inline_style_to_ratatui(chunk.style, theme, fallback_style),
            )
        })
        .collect()
}

fn inline_style_to_ratatui(
    inline: InlineStyle,
    theme: TerminalTheme,
    fallback_style: Style,
) -> Style {
    let mut style = fallback_style;
    if inline.strong {
        style = style.add_modifier(Modifier::BOLD);
    }
    if inline.emphasis {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if inline.inline_code {
        style = style.fg(theme.brand).bg(theme.user_msg_bg);
    }
    if inline.link {
        style = style.fg(theme.brand).add_modifier(Modifier::UNDERLINED);
    }
    style
}

fn wrap_segments(segments: &[(String, Style)], width: usize) -> Vec<ChatRenderLine> {
    if width == 0 {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut current_plain = String::new();
    let mut current_spans: Vec<(String, Style)> = Vec::new();
    let mut current_width = 0usize;

    for (text, style) in segments {
        for ch in text.chars() {
            if ch == '\n' {
                lines.push(finish_line(&mut current_plain, &mut current_spans));
                current_width = 0;
                continue;
            }

            let char_width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
            if current_width + char_width > width && !current_plain.is_empty() {
                lines.push(finish_line(&mut current_plain, &mut current_spans));
                current_width = 0;
            }

            current_plain.push(ch);
            push_span_text(&mut current_spans, ch.to_string(), *style);
            current_width += char_width;
        }
    }

    if !current_plain.is_empty() || current_spans.is_empty() {
        lines.push(finish_line(&mut current_plain, &mut current_spans));
    }

    lines
}

fn finish_line(plain_text: &mut String, spans: &mut Vec<(String, Style)>) -> ChatRenderLine {
    let line = ChatRenderLine::from_spans(std::mem::take(spans));
    plain_text.clear();
    line
}

fn push_span_text(spans: &mut Vec<(String, Style)>, text: String, style: Style) {
    if let Some((existing, existing_style)) = spans.last_mut() {
        if *existing_style == style {
            existing.push_str(&text);
            return;
        }
    }
    spans.push((text, style));
}

fn render_table(
    alignments: &[Alignment],
    headers: &[Vec<InlineChunk>],
    rows: &[Vec<Vec<InlineChunk>>],
    theme: TerminalTheme,
    width: usize,
    indent: usize,
) -> Vec<ChatRenderLine> {
    let available = width.saturating_sub(indent + GUTTER.chars().count() + 2 + TABLE_SAFETY_MARGIN);
    if available < MIN_TABLE_COL_WIDTH * headers.len().max(1) {
        return render_vertical_table(headers, rows, theme, width, indent);
    }

    let header_strings = headers
        .iter()
        .map(|cell| inline_chunks_plain_text(cell))
        .collect::<Vec<_>>();
    let row_strings = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| inline_chunks_plain_text(cell))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let col_count = header_strings
        .len()
        .max(row_strings.iter().map(Vec::len).max().unwrap_or(0));
    if col_count == 0 {
        return Vec::new();
    }

    let mut widths = vec![MIN_TABLE_COL_WIDTH; col_count];
    for (index, header) in header_strings.iter().enumerate() {
        widths[index] = widths[index].max(UnicodeWidthStr::width(header.as_str()));
    }
    for row in &row_strings {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(UnicodeWidthStr::width(cell.as_str()));
        }
    }

    let total = widths.iter().sum::<usize>() + col_count * 3 + 1;
    if total > available {
        let scaled_target = available.saturating_sub(col_count * 3 + 1).max(col_count);
        let total_width = widths.iter().sum::<usize>().max(1);
        widths = widths
            .into_iter()
            .map(|w| ((w * scaled_target) / total_width).max(MIN_TABLE_COL_WIDTH))
            .collect();
    }

    let estimated_lines = rows
        .iter()
        .flat_map(|row| row.iter().enumerate())
        .map(|(index, cell)| {
            let cell_width = widths
                .get(index)
                .copied()
                .unwrap_or(MIN_TABLE_COL_WIDTH)
                .max(1);
            let plain = UnicodeWidthStr::width(inline_chunks_plain_text(cell).as_str());
            plain.div_ceil(cell_width)
        })
        .max()
        .unwrap_or(1);
    if estimated_lines > TABLE_VERTICAL_THRESHOLD {
        return render_vertical_table(headers, rows, theme, width, indent);
    }

    let prefix = format!("{}{}", " ".repeat(indent), GUTTER);
    let mut rendered = Vec::new();
    rendered.push(ChatRenderLine::plain(
        format!("{} {}", prefix, build_table_border(&widths, '┌', '┬', '┐')),
        Style::default().fg(theme.subtle),
    ));
    rendered.push(ChatRenderLine::plain(
        format!(
            "{} {}",
            prefix,
            build_table_row(&header_strings, &widths, alignments, true)
        ),
        Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
    ));
    rendered.push(ChatRenderLine::plain(
        format!("{} {}", prefix, build_table_border(&widths, '├', '┼', '┤')),
        Style::default().fg(theme.subtle),
    ));
    for row in &row_strings {
        rendered.push(ChatRenderLine::plain(
            format!(
                "{} {}",
                prefix,
                build_table_row(row, &widths, alignments, false)
            ),
            Style::default().fg(theme.text),
        ));
    }
    rendered.push(ChatRenderLine::plain(
        format!("{} {}", prefix, build_table_border(&widths, '└', '┴', '┘')),
        Style::default().fg(theme.subtle),
    ));
    rendered
}

fn render_vertical_table(
    headers: &[Vec<InlineChunk>],
    rows: &[Vec<Vec<InlineChunk>>],
    theme: TerminalTheme,
    width: usize,
    indent: usize,
) -> Vec<ChatRenderLine> {
    let mut lines = Vec::new();
    let labels = headers
        .iter()
        .map(|cell| inline_chunks_plain_text(cell))
        .collect::<Vec<_>>();
    let separator = format!(
        "{}{} {}",
        " ".repeat(indent),
        GUTTER,
        "─".repeat(
            width
                .saturating_sub(indent + GUTTER.chars().count() + 1)
                .max(8)
        )
    );

    for (row_index, row) in rows.iter().enumerate() {
        if row_index > 0 {
            lines.push(ChatRenderLine::plain(
                separator.clone(),
                Style::default().fg(theme.subtle),
            ));
        }
        for (col_index, cell) in row.iter().enumerate() {
            let label = labels
                .get(col_index)
                .cloned()
                .unwrap_or_else(|| format!("Column {}", col_index + 1));
            let value = inline_chunks_plain_text(cell);
            let prefix = vec![
                (
                    format!("{}{}", " ".repeat(indent), GUTTER),
                    Style::default().fg(theme.subtle),
                ),
                (" ".to_string(), Style::default().fg(theme.text)),
                (
                    format!("{}: ", label),
                    Style::default()
                        .fg(theme.brand)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            let chunks = vec![InlineChunk {
                text: value,
                style: InlineStyle::default(),
            }];
            render_wrapped_inline_block(
                &mut lines,
                prefix,
                &chunks,
                theme,
                width,
                Style::default().fg(theme.text),
            );
        }
    }

    lines
}

fn build_table_border(widths: &[usize], left: char, cross: char, right: char) -> String {
    let mut line = String::new();
    line.push(left);
    for (index, width) in widths.iter().enumerate() {
        line.push_str(&"─".repeat(width + 2));
        line.push(if index + 1 == widths.len() {
            right
        } else {
            cross
        });
    }
    line
}

fn build_table_row(
    cells: &[String],
    widths: &[usize],
    alignments: &[Alignment],
    header: bool,
) -> String {
    let mut line = String::from("│");
    for (index, width) in widths.iter().enumerate() {
        let cell = cells.get(index).cloned().unwrap_or_default();
        let align = if header {
            Alignment::Center
        } else {
            alignments.get(index).copied().unwrap_or(Alignment::None)
        };
        let display_width = UnicodeWidthStr::width(cell.as_str());
        line.push(' ');
        line.push_str(&pad_aligned(&cell, display_width, *width, align));
        line.push(' ');
        line.push('│');
    }
    line
}

fn pad_aligned(
    content: &str,
    display_width: usize,
    target_width: usize,
    align: Alignment,
) -> String {
    let padding = target_width.saturating_sub(display_width);
    match align {
        Alignment::Center => {
            let left = padding / 2;
            format!(
                "{}{}{}",
                " ".repeat(left),
                content,
                " ".repeat(padding.saturating_sub(left))
            )
        }
        Alignment::Right => format!("{}{}", " ".repeat(padding), content),
        _ => format!("{}{}", content, " ".repeat(padding)),
    }
}

fn inline_chunks_plain_text(chunks: &[InlineChunk]) -> String {
    chunks
        .iter()
        .map(|chunk| match chunk.text.as_str() {
            "\n" => " ",
            text => text,
        })
        .collect::<Vec<_>>()
        .join("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_fast_path_keeps_single_paragraph() {
        let parsed = parse_markdown_cached("hello world");
        assert!(matches!(parsed.as_slice(), [MarkdownBlock::Paragraph(_)]));
    }

    #[test]
    fn render_assistant_markdown_formats_heading_and_list() {
        let theme = TerminalTheme::default();
        let lines = render_assistant_markdown("# Title\n\n- item", theme, 80);
        let plain = lines
            .iter()
            .map(|line| line.plain_text.clone())
            .collect::<Vec<_>>();
        assert!(plain.iter().any(|line| line.contains("Title")));
        assert!(plain.iter().any(|line| line.contains("- item")));
    }

    #[test]
    fn wrap_segments_breaks_long_lines() {
        let lines = wrap_segments(&[("abcdef".to_string(), Style::default())], 2);
        assert_eq!(
            lines
                .iter()
                .map(|line| line.plain_text.clone())
                .collect::<Vec<_>>(),
            vec!["ab", "cd", "ef"]
        );
    }

    #[test]
    fn render_table_falls_back_to_vertical_layout_when_narrow() {
        let theme = TerminalTheme::default();
        let headers = vec![
            vec![InlineChunk {
                text: "Name".to_string(),
                style: InlineStyle::default(),
            }],
            vec![InlineChunk {
                text: "Value".to_string(),
                style: InlineStyle::default(),
            }],
        ];
        let rows = vec![vec![
            vec![InlineChunk {
                text: "alpha".to_string(),
                style: InlineStyle::default(),
            }],
            vec![InlineChunk {
                text: "this is a very long value".to_string(),
                style: InlineStyle::default(),
            }],
        ]];

        let lines = render_table(
            &[Alignment::Left, Alignment::Left],
            &headers,
            &rows,
            theme,
            24,
            0,
        );
        let plain = lines
            .iter()
            .map(|line| line.plain_text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plain.contains("Name:"));
        assert!(plain.contains("Value:"));
    }
}
