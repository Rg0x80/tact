use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use crate::render::renderable::Renderable;
use crate::render::util::wrap_line;

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
    /// 高亮背景色。
    highlight_color: Color,
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
        highlight_color: Color,
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
            highlight_color,
        }
    }

    /// 构建带搜索高亮的行（与原先 log.rs 中 `build_search_highlighted_line` 逻辑一致）。
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
}

impl Renderable for TextCell {
    fn height(&self, _width: u16) -> u16 {
        self.cached_lines.len() as u16
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let wrap_width = area.width as usize;
        let lines: Vec<Line> = if self.is_search_match {
            self.build_highlighted_line(area.width)
        } else if self.is_selected {
            if let Some((ws, we)) = self.word_selection {
                // 词级选区：从 raw_text 切片并高亮
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
                    Span::styled(
                        word.to_string(),
                        Style::default().add_modifier(Modifier::REVERSED),
                    ),
                    Span::raw(after.to_string()),
                ]);
                wrap_line(&styled_line, wrap_width)
            } else {
                // 行级选区：给缓存行加 REVERSED
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
        } else {
            // 普通渲染
            self.cached_lines.clone()
        };

        // 绘制视觉行
        let mut y = area.y;
        for (i, line) in lines.iter().enumerate() {
            if y >= area.y + area.height {
                break;
            }
            let mut line = line.clone();
            // 首行添加 prefix（thinking 折叠指示符）
            if i == 0 {
                if let Some(ref prefix) = self.prefix {
                    if let Some(first) = line.spans.first_mut() {
                        first.content = format!("{}{}", prefix, first.content).into();
                    }
                }
            }
            // 将 line 的 spans 写入 buffer 的一行
            let mut x = area.x;
            for span in &line.spans {
                let content = span.content.as_ref();
                for ch in content.chars() {
                    if x < area.x + area.width {
                        buf[(x, y)].set_char(ch).set_style(span.style);
                        x += 1;
                    }
                }
            }
            y += 1;
        }
    }
}
