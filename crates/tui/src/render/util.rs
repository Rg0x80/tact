use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// 将单行文本在指定显示宽度处拆分，返回 (前缀, 剩余)。
/// 前缀的显示宽度 ≤ max_width。
fn split_at_display_width(text: &str, max_width: usize) -> (&str, &str) {
    if text.is_empty() || max_width == 0 {
        return ("", text);
    }
    let mut current_width = 0;
    for (i, c) in text.char_indices() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if current_width + cw > max_width {
            return (&text[..i], &text[i..]);
        }
        current_width += cw;
    }
    (text, "")
}

/// 将一条 styled Line 按显示宽度拆分为多条不超出 max_width 的 Line。
/// 子行继承首段 span 的样式；对于多 span 行保留主导样式。
pub(crate) fn wrap_line(line: &Line<'_>, max_width: usize) -> Vec<Line<'static>> {
    let text: String = line
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<Vec<_>>()
        .concat();
    let base_style = line.spans.first().map(|s| s.style).unwrap_or_default();

    let mut result = Vec::new();
    for text_line in text.lines() {
        if text_line.is_empty() {
            result.push(Line::from(Span::styled("", base_style)));
            continue;
        }
        let w = UnicodeWidthStr::width(text_line);
        if w <= max_width {
            result.push(Line::from(Span::styled(text_line.to_string(), base_style)));
            continue;
        }
        let mut remaining = text_line;
        while !remaining.is_empty() {
            let (seg, rest) = split_at_display_width(remaining, max_width);
            if seg.is_empty() {
                if let Some(c) = rest.chars().next() {
                    let mut s = String::new();
                    s.push(c);
                    result.push(Line::from(Span::styled(s, base_style)));
                    remaining = &rest[c.len_utf8()..];
                } else {
                    break;
                }
            } else {
                result.push(Line::from(Span::styled(seg.to_string(), base_style)));
                remaining = rest;
            }
        }
    }
    if result.is_empty() {
        result.push(Line::from(Span::styled("", base_style)));
    }
    result
}
