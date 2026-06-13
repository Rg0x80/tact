use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthChar;
use ratatui::text::{Line, Span};
use crate::render::renderable::Renderable;
use crate::render::util::wrap_line;

/// 将单行 span 写入 buffer。
fn render_line(line: &Line, x: u16, y: u16, width: u16, buf: &mut Buffer) {
    let mut col = x;
    for span in &line.spans {
        for ch in span.content.chars() {
            if col < x + width {
                buf[(col, y)].set_char(ch).set_style(span.style);
                col += UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
            }
        }
    }
}

/// 单条日志消息的渲染单元。
/// 预缓存折行结果，支持搜索高亮和鼠标选区。
pub(crate) struct TextCell {
    /// 预折行的视觉行（普通渲染时直接 clone）。
    cached_lines: Vec<Line<'static>>,
    /// 原始文本（搜索高亮时用）。
    raw_text: String,
    /// 搜索词。
    search_term: String,
    /// 是否为搜索命中行。
    is_search_match: bool,
    /// 是否被鼠标选中。
    is_selected: bool,
    /// 词级选区 (start_byte, end_byte)，None 表示行级选区。
    word_selection: Option<(usize, usize)>,
    /// 首行前缀（thinking block 折叠指示符）。
    prefix: Option<String>,
    /// 普通前景色。
    fg_color: Color,
}

impl TextCell {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        cached_lines: Vec<Line<'static>>,
        raw_text: String,
        search_term: String,
        is_search_match: bool,
        is_selected: bool,
        word_selection: Option<(usize, usize)>,
        prefix: Option<String>,
        fg_color: Color,
    ) -> Self {
        TextCell {
            cached_lines,
            raw_text,
            search_term,
            is_search_match,
            is_selected,
            word_selection,
            prefix,
            fg_color,
        }
    }

    /// 构建渲染用的视觉行列表（根据状态选择搜索高亮/选区/缓存）。
    fn build_lines(&self, wrap_width: u16) -> Vec<Line<'_>> {
        if self.is_search_match {
            return self.build_highlighted_line(wrap_width);
        }
        if self.is_selected {
            if let Some((ws, we)) = self.word_selection {
                return self.build_word_selected_lines(wrap_width, ws, we);
            }
            return self.build_line_selected_lines();
        }
        self.cached_lines.clone()
    }

    fn build_highlighted_line(&self, wrap_width: u16) -> Vec<Line<'static>> {
        let lower_raw = self.raw_text.to_lowercase();
        let lower_term = self.search_term.to_lowercase();
        let mut spans = Vec::new();
        let mut last_idx = 0;

        for (match_idx, _) in lower_raw.match_indices(&lower_term) {
            if match_idx > last_idx {
                spans.push(Span::styled(
                    self.raw_text[last_idx..match_idx].to_string(),
                    Style::default().fg(self.fg_color),
                ));
            }
            let end_idx = match_idx + lower_term.len();
            spans.push(Span::styled(
                self.raw_text[match_idx..end_idx].to_string(),
                Style::default().bg(Color::Yellow).fg(Color::Black),
            ));
            last_idx = end_idx;
        }
        if last_idx < self.raw_text.len() {
            spans.push(Span::styled(
                self.raw_text[last_idx..].to_string(),
                Style::default().fg(self.fg_color),
            ));
        }

        let mut line = Line::from(spans);
        if self.is_selected {
            for span in line.spans.iter_mut() {
                span.style = span.style.add_modifier(Modifier::REVERSED);
            }
        }
        wrap_line(&line, wrap_width as usize)
    }

    fn build_word_selected_lines(&self, wrap_width: u16, ws: usize, we: usize) -> Vec<Line<'static>> {
        let raw = &self.raw_text;
        let w_start = raw.floor_char_boundary(ws.min(raw.len()));
        let w_end = raw.floor_char_boundary(we.min(raw.len()));
        let (w_start, w_end) = if w_end < w_start {
            (w_end, w_start)
        } else {
            (w_start, w_end)
        };
        let before = &raw[..w_start];
        let word = &raw[w_start..w_end];
        let after = &raw[w_end..];
        let styled_line = Line::from(vec![
            Span::raw(before.to_string()),
            Span::styled(word.to_string(), Style::default().add_modifier(Modifier::REVERSED)),
            Span::raw(after.to_string()),
        ]);
        wrap_line(&styled_line, wrap_width as usize)
    }

    fn build_line_selected_lines(&self) -> Vec<Line<'static>> {
        self.cached_lines
            .iter()
            .map(|line| {
                let mut line = line.clone();
                for span in line.spans.iter_mut() {
                    span.style = span.style.add_modifier(Modifier::REVERSED);
                }
                line
            })
            .collect()
    }
}

impl Renderable for TextCell {
    fn height(&self, _width: u16) -> u16 {
        self.cached_lines.len() as u16
    }

    fn render_partial(&self, area: Rect, buf: &mut Buffer, skip_lines: usize) {
        let lines = self.build_lines(area.width);
        let mut y = area.y;
        for (i, line) in lines.iter().enumerate().skip(skip_lines) {
            if y >= area.y + area.height {
                break;
            }
            let mut line = line.clone();
            // 只在 cell 首行（i == 0）添加 prefix
            if i == 0 {
                if let Some(ref prefix) = self.prefix {
                    if let Some(first) = line.spans.first_mut() {
                        first.content = format!("{}{}", prefix, first.content).into();
                    }
                }
            }
            render_line(&line, area.x, y, area.width, buf);
            y += 1;
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.render_partial(area, buf, 0);
    }
}
