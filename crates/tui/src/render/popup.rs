use crate::state::{App, PALETTE_COMMANDS};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Scrollbar, ScrollbarState, Wrap},
};

// ── Lightweight popups ──

/// 渲染居中的命令面板弹出层，支持过滤和高亮当前选中项。
pub(crate) fn render_command_palette(frame: &mut Frame, area: Rect, app: &App) {
    let filter = app.cmd_line.to_lowercase();
    let filtered: Vec<(usize, &(&str, &str))> = PALETTE_COMMANDS
        .iter()
        .enumerate()
        .filter(|(_, (cmd, desc))| {
            filter.is_empty()
                || cmd.to_lowercase().contains(&filter)
                || desc.to_lowercase().contains(&filter)
        })
        .collect();

    let count = filtered.len().max(1) as u16;
    let popup_width = 44u16;
    let popup_height = count + 4;
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(app.msgs().palette_title.replace("{}", &app.cmd_line))
        .style(Style::default().bg(app.theme.bottom_bar_bg));
    frame.render_widget(block.clone(), popup_area);

    let inner = Rect::new(
        popup_area.x + 1,
        popup_area.y + 1,
        popup_area.width.saturating_sub(2),
        popup_area.height.saturating_sub(2),
    );

    let items: Vec<ListItem> = if filtered.is_empty() {
        vec![ListItem::new(Span::styled(
            app.msgs().palette_empty,
            Style::default().fg(Color::Gray),
        ))]
    } else {
        filtered
            .iter()
            .enumerate()
            .map(|(i, (_orig_idx, (cmd, _desc)))| {
                let is_selected = i == app.palette_selected.min(filtered.len().saturating_sub(1));
                let style = if is_selected {
                    Style::default().bg(app.theme.highlight).fg(Color::White)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                let text = format!("  {:<12} {}", cmd, app.localize_cmd_desc(cmd));
                ListItem::new(Span::styled(text, style))
            })
            .collect()
    };

    let list = List::new(items).block(Block::default());
    frame.render_widget(list, inner);
}

/// 渲染选择弹窗，居中显示 prompt 和选项列表。
pub(crate) fn render_select_popup(frame: &mut Frame, area: Rect, app: &App) {
    let count = app.select.options.len().max(1) as u16;
    let popup_width = 50u16.min(area.width.saturating_sub(4));
    let popup_height = (count + 4).min(area.height.saturating_sub(4));
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", app.select.prompt))
        .style(Style::default().bg(app.theme.bottom_bar_bg));
    frame.render_widget(block.clone(), popup_area);

    let inner = Rect::new(
        popup_area.x + 1,
        popup_area.y + 1,
        popup_area.width.saturating_sub(2),
        popup_area.height.saturating_sub(2),
    );

    let items: Vec<ListItem> = if app.select.options.is_empty() {
        vec![ListItem::new(Span::styled(
            app.msgs().select_empty,
            Style::default().fg(Color::Gray),
        ))]
    } else {
        let selected = app.select.selected.min(app.select.options.len().saturating_sub(1));
        app.select
            .options
            .iter()
            .enumerate()
            .map(|(i, opt)| {
                let is_selected = i == selected;
                let style = if is_selected {
                    Style::default().bg(app.theme.highlight).fg(Color::White)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                let prefix = if is_selected { app.msgs().select_arrow } else { "  " };
                ListItem::new(Span::styled(format!("{}{}", prefix, opt), style))
            })
            .collect()
    };

    let list = List::new(items).block(Block::default());
    frame.render_widget(list, inner);
}

/// 渲染任务历史面板，按时间倒序展示。
pub(crate) fn render_history_panel(frame: &mut Frame, area: Rect, app: &mut App) {
    let items: Vec<ListItem> = app
        .task_history
        .iter()
        .rev()
        .enumerate()
        .map(|(_i, entry)| {
            let mut text = format!("[{}] {}", entry.timestamp, entry.task);
            if !entry.summary.is_empty() {
                text.push_str(&format!(" -> {}", entry.summary));
            }
            ListItem::new(text).style(Style::default().fg(app.theme.accent))
        })
        .collect();
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(app.theme.border))
            .title(app.msgs().history_title),
    );
    frame.render_widget(list, area);
}

/// 渲染帮助面板，展示所有可用的键盘快捷键和鼠标操作。
pub(crate) fn render_help_panel(frame: &mut Frame, area: Rect, app: &mut App) {
    let msgs = app.msgs();
    let help_text = vec![
        Line::from(msgs.help_header_shortcuts),
        Line::from(""),
        Line::from(msgs.help_normal_header),
        Line::from(msgs.help_tab),
        Line::from(msgs.help_e),
        Line::from(msgs.help_jk),
        Line::from(msgs.help_gg),
        Line::from(msgs.help_G),
        Line::from(msgs.help_y),
        Line::from(msgs.help_t),
        Line::from(msgs.help_slash),
        Line::from(msgs.help_nN),
        Line::from(msgs.help_colon),
        Line::from(""),
        Line::from(msgs.help_insert_header),
        Line::from(msgs.help_type_task),
        Line::from(msgs.help_ctrl_z),
        Line::from(""),
        Line::from(msgs.help_global_header),
        Line::from(msgs.help_yn),
        Line::from(msgs.help_ctrl_h),
        Line::from(msgs.help_ctrl_t),
        Line::from(msgs.help_ctrl_l),
        Line::from(msgs.help_ctrl_qmark),
        Line::from(msgs.help_q),
        Line::from(""),
        Line::from(msgs.help_mouse_header),
        Line::from(msgs.help_click_drag),
        Line::from(msgs.help_scroll),
        Line::from(msgs.help_y_copy),
    ];
    let para = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title(app.msgs().help_title))
        .style(Style::default().fg(app.theme.fg).bg(app.theme.bg));
    frame.render_widget(para, area);
}

// ── Overlay popups ──

/// 渲染 Thinking 弹窗，显示完整推理内容并支持滚动。
pub(crate) fn render_thinking_popup(frame: &mut Frame, area: Rect, app: &mut App) {
    let popup = match &app.thinking.popup {
        Some(p) => p,
        None => return,
    };
    let block = &app.thinking.blocks[popup.block_idx];
    let raw_total = block.end_idx.saturating_sub(block.title_idx);
    if raw_total == 0 { return; }

    let styled_lines = &block.cached_markdown;
    let total = styled_lines.len();
    if total == 0 { return; }

    let popup_width = (area.width as f32 * 0.8) as u16;
    let popup_height = (area.height as f32 * 0.8) as u16;
    let popup_x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let content_height = popup_height.saturating_sub(3) as usize;
    let max_scroll = total.saturating_sub(1);
    let scroll = (popup.scroll as usize).min(max_scroll);
    let start_line = scroll;
    let end_line = (scroll + content_height).min(total);

    let mut text = Text::default();
    let title_style = Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD);
    text.push_line(Line::from(Span::styled(
        format!("{} ({} markdown lines, {} raw)", popup.title, total, raw_total),
        title_style,
    )));
    text.push_line(Line::from(""));
    for line in &styled_lines[start_line..end_line] {
        text.push_line(line.clone());
    }

    let para = Paragraph::new(text)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(app.msgs().thinking_popup_title)
            .title_bottom(Line::from(vec![
                Span::styled(app.msgs().popup_copy_hint, Style::default().fg(app.theme.accent)),
                Span::styled(app.msgs().popup_close_hint, Style::default().fg(app.theme.accent)),
                Span::styled(app.msgs().popup_scroll_hint, Style::default().fg(app.theme.accent)),
            ]))
            .style(Style::default().fg(app.theme.fg).bg(app.theme.bg)))
        .wrap(Wrap { trim: false });

    frame.render_widget(para, popup_area);

    let scrollbar = Scrollbar::default()
        .orientation(ratatui::widgets::ScrollbarOrientation::VerticalRight);
    let mut state = ScrollbarState::new(total).viewport_content_length(content_height).position(scroll);
    frame.render_stateful_widget(scrollbar, popup_area, &mut state);

    app.mouse.thinking_popup_area = popup_area;
}

/// 渲染文件内容弹窗，显示写入文件的完整内容并支持滚动。
pub(crate) fn render_diff_popup(frame: &mut Frame, area: Rect, app: &mut App) {
    let popup = match &app.diff_popup {
        Some(p) => p,
        None => return,
    };

    let lines: Vec<&str> = popup.content.lines().collect();
    let total = lines.len();
    if total == 0 { return; }

    let popup_width = (area.width as f32 * 0.8).max(40.0) as u16;
    let popup_height = (area.height as f32 * 0.8).max(10.0) as u16;
    let popup_x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let content_height = popup_height.saturating_sub(3) as usize;
    let max_scroll = total.saturating_sub(1);
    let scroll = (popup.scroll as usize).min(max_scroll);
    let start_line = scroll;
    let end_line = (start_line + content_height).min(total);

    let num_width = (total + 1).to_string().len().max(3);
    let code_width = (popup_width as usize).saturating_sub(4 + num_width);
    let num_style = Style::default().fg(app.theme.border);
    let text_style = Style::default().fg(app.theme.fg);

    let mut text = Text::default();
    for i in start_line..end_line {
        let num = format!("{:>nw$}", i + 1, nw = num_width);
        let trimmed: String = lines[i].chars().take(code_width).collect();
        text.push_line(Line::from(vec![
            Span::styled(format!(" {} ", num), num_style),
            Span::styled(trimmed, text_style),
        ]));
    }

    let para = Paragraph::new(text)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(app.msgs().diff_popup_title.replace("{}", &popup.file_path))
            .title_bottom(Line::from(vec![
                Span::styled(app.msgs().popup_copy_hint, Style::default().fg(app.theme.accent)),
                Span::styled(app.msgs().popup_close_hint, Style::default().fg(app.theme.accent)),
                Span::styled(app.msgs().popup_scroll_hint, Style::default().fg(app.theme.accent)),
            ]))
            .style(Style::default().fg(app.theme.fg).bg(app.theme.bg)))
        .wrap(Wrap { trim: false });

    frame.render_widget(para, popup_area);

    let scrollbar = Scrollbar::default()
        .orientation(ratatui::widgets::ScrollbarOrientation::VerticalRight);
    let mut state = ScrollbarState::new(total).viewport_content_length(content_height).position(scroll);
    frame.render_stateful_widget(scrollbar, popup_area, &mut state);

    app.mouse.diff_popup_area = popup_area;
}
