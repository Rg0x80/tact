use crate::theme::Theme;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// 自定义 StyleSheet，为代码块提供深色背景和等宽风格。
#[derive(Clone, Copy, Debug, Default)]
struct TuiStyleSheet;

impl tui_markdown::StyleSheet for TuiStyleSheet {
    fn heading(&self, level: u8) -> Style {
        match level {
            1 => Style::new().on_cyan().bold().underlined(),
            2 => Style::new().cyan().bold(),
            3 => Style::new().cyan().bold().italic(),
            4 => Style::new().light_cyan().italic(),
            5 => Style::new().light_cyan().italic(),
            _ => Style::new().light_cyan().italic(),
        }
    }

    fn code(&self) -> Style {
        Style::new()
            .fg(Color::Rgb(220, 220, 220))
            .bg(Color::Rgb(30, 35, 50))
    }

    fn link(&self) -> Style {
        Style::new().blue().underlined()
    }

    fn blockquote(&self) -> Style {
        Style::new().green()
    }

    fn heading_meta(&self) -> Style {
        Style::new().dim()
    }

    fn metadata_block(&self) -> Style {
        Style::new().light_yellow()
    }
}

/// 使用 tui-markdown 将 Markdown 文本渲染为 ratatui 的 Line 列表和原始文本列表。
/// 对代码块进行后处理：添加顶部分隔线（含语言标签）、行号和底部分隔线。
pub(crate) fn render_markdown_tui(text: &str) -> (Vec<Line<'static>>, Vec<String>) {
    // NOTE: Do NOT call process_hyperlinks here — ratatui strips raw ESC sequences
    // (including OSC 8) from Span text, causing broken ]8;; garbage to appear on screen.
    // Plain URLs render fine in the TUI and can be copied via clipboard.
    let options = tui_markdown::Options::new(TuiStyleSheet);
    let tui_text = tui_markdown::from_str_with_options(&text, &options);
    let mut styled_lines: Vec<Line<'static>> = tui_text
        .lines
        .into_iter()
        .map(|line| {
            let spans: Vec<Span<'static>> = line
                .spans
                .into_iter()
                .map(|s| Span::styled(s.content.into_owned(), s.style))
                .collect();
            let mut new_line = Line::from(spans).style(line.style);
            if let Some(alignment) = line.alignment {
                new_line = new_line.alignment(alignment);
            }
            new_line
        })
        .collect();
    let raw_lines: Vec<String> = styled_lines.iter().map(|l| l.to_string()).collect();

    // Post-process: apply background to code block content lines
    apply_code_background(&mut styled_lines, &raw_lines);

    let raw_lines: Vec<String> = styled_lines.iter().map(|l| l.to_string()).collect();
    (styled_lines, raw_lines)
}

/// 为代码块内容行添加统一的深色背景，保持 tui-markdown 原生语法高亮。
/// ``` 标记行保持原样（由 tui-markdown 渲染）。
fn apply_code_background(lines: &mut Vec<Line<'static>>, raw: &[String]) {
    let code_bg = Color::Rgb(30, 35, 50);
    let code_fg = Color::Rgb(200, 200, 210);

    let mut i = 0;
    while i < raw.len() {
        let trimmed = raw[i].trim();
        if trimmed.starts_with("```") {
            // 查找闭合的 ```
            let mut end_marker = None;
            let mut j = i + 1;
            while j < raw.len() {
                if raw[j].trim() == "```" {
                    end_marker = Some(j);
                    break;
                }
                j += 1;
            }

            if let Some(end) = end_marker {
                // 内容行加背景（``` 标记行保持原样）
                for line_idx in (i + 1)..end {
                    let mut spans: Vec<Span<'static>> = Vec::new();
                    for span in &lines[line_idx].spans {
                        let mut style = span.style;
                        if style.fg.is_none() {
                            style = style.fg(code_fg);
                        }
                        style = style.bg(code_bg);
                        spans.push(Span::styled(span.content.clone(), style));
                    }
                    if !spans.is_empty() {
                        lines[line_idx] = Line::from(spans);
                    }
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
}

/// 判断一行是否为 Markdown 水平分隔线（---, ***, ___，允许空格穿插）。
pub(crate) fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 {
        return false;
    }
    let marks: Vec<char> = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    if marks.len() < 3 {
        return false;
    }
    let first = marks[0];
    if first != '-' && first != '*' && first != '_' {
        return false;
    }
    marks.iter().all(|&c| c == first)
}

/// 把 Markdown 表格原始行解析为列对齐的 ratatui Line。
pub(crate) fn format_table(lines: &[String], theme: &Theme) -> (Vec<Line<'static>>, Vec<String>) {
    let rows: Vec<Vec<String>> = lines
        .iter()
        .map(|line| {
            let mut cells: Vec<String> = line.split('|').map(|s| s.trim().to_string()).collect();
            // 去掉开头和结尾因行首/行尾 | 产生的空单元格
            if cells.first().map(|s| s.is_empty()).unwrap_or(false) {
                cells.remove(0);
            }
            if cells.last().map(|s| s.is_empty()).unwrap_or(false) {
                cells.pop();
            }
            cells
        })
        .collect();

    if rows.is_empty() {
        return (Vec::new(), Vec::new());
    }

    // 计算每列最大宽度
    let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut col_widths = vec![0; col_count];
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            if i < col_widths.len() {
                col_widths[i] = col_widths[i].max(cell.len());
            }
        }
    }

    let mut styled_lines = Vec::new();
    let mut raw_lines = Vec::new();

    for (row_idx, row) in rows.iter().enumerate() {
        let mut cells = Vec::new();
        for (i, cell) in row.iter().enumerate() {
            let width = col_widths.get(i).copied().unwrap_or(0);
            cells.push(format!(" {:width$} ", cell, width = width));
        }
        let line_text = format!("|{}|", cells.join("|"));

        // 检测分隔行（所有单元格只包含 -、: 和空白）
        let is_sep = row.iter().all(|c| {
            c.chars()
                .all(|ch| ch == '-' || ch == ':' || ch.is_whitespace())
        });

        // 分隔行不渲染（跳过），避免将数据行误着色为 Gray
        if is_sep {
            continue;
        }

        let style = if row_idx == 0 {
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(theme.accent)
        } else {
            Style::default().fg(theme.fg)
        };

        styled_lines.push(Line::from(Span::styled(line_text.clone(), style)));
        raw_lines.push(line_text);
    }

    (styled_lines, raw_lines)
}
