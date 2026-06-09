use crate::state::{App, InputMode};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{Block, BorderType, Borders, Paragraph},
};
use unicode_width::UnicodeWidthStr;

/// 渲染命令行输入（Search / Palette 模式）。
pub(crate) fn render_command_line(frame: &mut Frame, area: Rect, app: &App) {
    let prefix = match app.input_mode {
        InputMode::Search => "/",
        _ => "",
    };
    let content = format!("{}{}", prefix, app.cmd_line);
    let input_para = Paragraph::new(content)
        .style(
            Style::default()
                .fg(app.theme.input_box_fg)
                .bg(app.theme.input_box_bg),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(app.msgs().command_title),
        );
    frame.render_widget(input_para, area);
    let cmd_width = UnicodeWidthStr::width(app.cmd_line.as_str()) as u16;
    let cursor_pos = (area.x + 2 + cmd_width).min(area.x + area.width - 2);
    frame.set_cursor_position((cursor_pos, area.y + 1));
}

/// 渲染主输入框（Insert 模式），或转接到命令行渲染。
pub(crate) fn render_input_box(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.input_mode == InputMode::Search || app.input_mode == InputMode::Palette {
        render_command_line(frame, area, app);
        return;
    }

    let mut cursor_line = 0;
    let mut cursor_col = 0;
    for (i, c) in app.input.char_indices() {
        if i >= app.input_cursor {
            break;
        }
        if c == '\n' {
            cursor_line += 1;
            cursor_col = 0;
        } else {
            cursor_col += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        }
    }

    let visible_lines = area.height.saturating_sub(2) as usize;

    if cursor_line < app.input_scroll as usize {
        app.input_scroll = cursor_line as u16;
    } else if cursor_line >= app.input_scroll as usize + visible_lines {
        app.input_scroll = (cursor_line - visible_lines + 1) as u16;
    }

    let lines: Vec<&str> = app.input.split('\n').collect();
    let start = app.input_scroll as usize;
    let end = (start + visible_lines).min(lines.len());
    let display_text = lines[start..end].join("\n");

    let input_para = Paragraph::new(display_text)
        .style(
            Style::default()
                .fg(app.theme.input_box_fg)
                .bg(app.theme.input_box_bg),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(app.msgs().input_box_title),
        );
    frame.render_widget(input_para, area);

    let cursor_x = area.x + 1 + cursor_col as u16;
    let cursor_y = area.y + 1 + (cursor_line - app.input_scroll as usize) as u16;
    frame.set_cursor_position((cursor_x, cursor_y));
}
