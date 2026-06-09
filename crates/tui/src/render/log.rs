use crate::render::util::wrap_line;
use crate::state::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarState},
};

/// 构建带搜索高亮的 Line。
fn build_search_highlighted_line<'a>(
    raw: &'a str,
    term: &str,
    base_fg: Color,
    is_selected: bool,
) -> Line<'a> {
    let mut spans = Vec::new();
    let lower_raw = raw.to_lowercase();
    let lower_term = term.to_lowercase();
    let mut last_idx = 0;

    for (match_idx, _) in lower_raw.match_indices(&lower_term) {
        if match_idx > last_idx {
            spans.push(Span::styled(
                &raw[last_idx..match_idx],
                Style::default().fg(base_fg),
            ));
        }
        let end_idx = match_idx + lower_term.len();
        spans.push(Span::styled(
            &raw[match_idx..end_idx],
            Style::default().bg(Color::Yellow).fg(Color::Black),
        ));
        last_idx = end_idx;
    }
    if last_idx < raw.len() {
        spans.push(Span::styled(&raw[last_idx..], Style::default().fg(base_fg)));
    }

    let mut line = Line::from(spans);
    if is_selected {
        for span in line.spans.iter_mut() {
            span.style = span.style.add_modifier(Modifier::REVERSED);
        }
    }
    line
}

/// 渲染 Log 面板，支持长行自动折行、滚动、搜索高亮和鼠标选择高亮。
pub(crate) fn render_log_panel(frame: &mut Frame, area: Rect, app: &mut App) {
    app.log_scroll.height = area.height.saturating_sub(2);
    let visible_height = app.log_scroll.height as usize;
    let max_width = area.width.saturating_sub(2) as usize;
    let wrap_width = if max_width > 0 { max_width } else { 1 };

    // ---- visible_indices ----
    let indices_stale = app.log_scroll.visible_indices_ver != app.messages.len();
    if indices_stale {
        app.visible_indices.clear();
        app.log_scroll.phys_to_logical_cache.clear();
        app.log_scroll
            .phys_to_logical_cache
            .resize(app.messages.len(), None);
        let mut total_logical = 0;
        for phys in 0..app.messages.len() {
            if app.is_message_visible(phys) {
                app.visible_indices.push(phys);
                app.log_scroll.phys_to_logical_cache[phys] = Some(total_logical);
                total_logical += 1;
            }
        }
        app.log_scroll.visible_indices_ver = app.messages.len();
    }
    let mut total_logical = app.visible_indices.len();
    if !app.stream.buffer.is_empty() {
        total_logical += 1;
    }

    // ---- Phase 1: wrap cache ----
    let cache_valid = app.log_scroll.visual_cache_ver == app.messages.len()
        && app.log_scroll.visual_cache_width == wrap_width as u16;

    if !cache_valid {
        app.log_scroll.visual_cache.clear();
        app.log_scroll.visual_start_cache.clear();
        app.log_scroll.visual_start_cache.push(0);

        for logical_i in 0..total_logical {
            let line = if let Some(&phys_idx) = app.visible_indices.get(logical_i) {
                let base = &app.messages[phys_idx];
                if base.spans.is_empty() {
                    Line::default()
                } else {
                    base.clone()
                }
            } else {
                Line::from(Span::styled(app.stream.buffer.as_str(), app.theme.accent))
            };
            let wrapped = wrap_line(&line, wrap_width);
            app.log_scroll.visual_cache.extend(wrapped);
            app.log_scroll
                .visual_start_cache
                .push(app.log_scroll.visual_cache.len());
        }
        app.log_scroll.visual_cache_width = wrap_width as u16;
        app.log_scroll.visual_cache_ver = app.messages.len();
    }

    // ---- Phase 2: clip viewport ----
    let total_visual = *app.log_scroll.visual_start_cache.last().unwrap_or(&0);
    let effective_max_logical = if total_visual <= visible_height {
        0
    } else {
        let target = total_visual - visible_height;
        match app.log_scroll.visual_start_cache.binary_search(&target) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        }
    };
    let max_scroll = effective_max_logical as u16;
    if app.log_scroll.offset > max_scroll {
        app.log_scroll.offset = max_scroll;
    }

    let logical_scroll = app.log_scroll.offset as usize;
    let vs_cache = &app.log_scroll.visual_start_cache;
    let max_visual_scroll = total_visual.saturating_sub(visible_height);
    let visual_scroll = if logical_scroll < vs_cache.len() {
        vs_cache[logical_scroll].min(max_visual_scroll)
    } else {
        max_visual_scroll
    };
    let end_visual = (visual_scroll + visible_height).min(total_visual);
    let visual_scroll = if end_visual >= total_visual && total_visual > visible_height {
        max_visual_scroll
    } else {
        visual_scroll
    };
    let end_visual = (visual_scroll + visible_height).min(total_visual);

    // ---- Phase 3: render visible lines ----
    let logical_start = match vs_cache.binary_search(&visual_scroll) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let logical_end = match vs_cache.binary_search(&end_visual) {
        Ok(i) => i,
        Err(i) => i.min(total_logical),
    };

    let has_search = !app.search.term.is_empty();
    let has_selection = app.mouse.log_selection.is_some();

    let mut text = Text::default();
    for logical_i in logical_start..logical_end {
        let cache_start = vs_cache[logical_i];
        let cache_end = vs_cache[logical_i + 1];
        if cache_end <= visual_scroll || cache_start >= end_visual {
            continue;
        }

        let vis_start_in_block = visual_scroll.max(cache_start);
        let vis_end_in_block = end_visual.min(cache_end);
        let block_offset = vis_start_in_block - cache_start;
        let block_count = vis_end_in_block - vis_start_in_block;

        if block_count == 0 {
            continue;
        }

        let is_match = has_search && app.search.matches.contains(&logical_i);
        let is_selected = has_selection
            && app
                .mouse
                .log_selection
                .map(|(s, e)| logical_i >= s.min(e) && logical_i <= s.max(e))
                .unwrap_or(false);

        let phys_idx = app.visible_indices.get(logical_i).copied();
        let cached_slice = &app.log_scroll.visual_cache[cache_start..cache_end];

        if !has_search && !is_selected {
            for line in &cached_slice[block_offset..block_offset + block_count] {
                text.push_line(line.clone());
            }
        } else if has_search && is_match {
            if let Some(phys) = phys_idx {
                let raw = &app.raw_messages[phys];
                let line =
                    build_search_highlighted_line(raw, &app.search.term, app.theme.fg, is_selected);
                let wrapped = wrap_line(&line, wrap_width);
                let local_start = block_offset.min(wrapped.len());
                let local_end = (block_offset + block_count).min(wrapped.len());
                for wline in &wrapped[local_start..local_end] {
                    text.push_line(wline.clone());
                }
            }
        } else {
            // 选择高亮 / 无高亮回退
            let mut push_count = 0;
            let word_sel = app
                .mouse
                .log_word_selection
                .filter(|_| is_selected && phys_idx.is_some());
            for (local_i, cached_line) in cached_slice.iter().enumerate().skip(block_offset) {
                if push_count >= block_count {
                    break;
                }
                if let Some((ws, we)) = word_sel {
                    let raw = &app.raw_messages[phys_idx.unwrap()];
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
                    let wrapped = wrap_line(&styled_line, wrap_width);
                    let local_start = block_offset.min(wrapped.len());
                    let local_end = (block_offset + block_count).min(wrapped.len());
                    for wline in &wrapped[local_start..local_end] {
                        text.push_line(wline.clone());
                        push_count += 1;
                    }
                } else {
                    let mut line = cached_line.clone();
                    if is_selected {
                        for span in line.spans.iter_mut() {
                            span.style = span.style.add_modifier(Modifier::REVERSED);
                        }
                    }
                    if local_i == 0 {
                        if let Some(phys) = phys_idx {
                            for block in &app.thinking.blocks {
                                if block.title_idx == phys {
                                    let total = block.end_idx.saturating_sub(block.title_idx);
                                    let indicator = if total > 3 {
                                        app.msgs()
                                            .scroll_indicator_tmpl
                                            .replacen("{}", &total.min(3).to_string(), 1)
                                            .replacen("{}", &total.to_string(), 1)
                                    } else {
                                        String::new()
                                    };
                                    if let Some(first) = line.spans.first_mut() {
                                        first.content =
                                            format!("{}{}", indicator, first.content).into();
                                    }
                                    break;
                                }
                            }
                        }
                    }
                    text.push_line(line);
                    push_count += 1;
                }
            }
        }
    }

    let log_paragraph = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(app.theme.border))
            .title(app.msgs().log_title)
            .style(Style::default().bg(app.theme.bg)),
    );
    frame.render_widget(log_paragraph, area);

    // ---- Card overlay: thinking blocks ----
    let vs_cache = &app.log_scroll.visual_start_cache;
    for block in &app.thinking.blocks {
        let Some(title_logical) = app.phys_to_logical_fast(block.title_idx) else {
            continue;
        };
        let blank_after_phys = block.end_idx + 1;
        let Some(blank_after_logical) = app.phys_to_logical_fast(blank_after_phys) else {
            continue;
        };
        if title_logical >= vs_cache.len() || blank_after_logical >= vs_cache.len() {
            continue;
        }
        let vis_card_top = vs_cache[title_logical];
        let vis_card_bottom = vs_cache[blank_after_logical];
        let vis_range_end = visual_scroll + visible_height;
        if vis_card_bottom <= visual_scroll || vis_card_top >= vis_range_end {
            continue;
        }
        let y_top = (vis_card_top.saturating_sub(visual_scroll)) as u16;
        let y_bot = (vis_card_bottom.saturating_sub(visual_scroll)).min(visible_height as _) as u16;
        if y_bot <= y_top {
            continue;
        }

        let total_lines = block.end_idx.saturating_sub(block.title_idx);
        let card_style = Style::default().fg(Color::Rgb(140, 140, 220));
        let visible_count = total_lines.min(3);
        let showing_from = total_lines.saturating_sub(visible_count);
        let msgs = app.msgs();
        let card_block = Block::default()
            .borders(Borders::ALL)
            .border_style(card_style)
            .style(Style::default().bg(app.theme.bg))
            .title(
                msgs.thinking_card_title
                    .replacen("{}", &total_lines.to_string(), 1)
                    .replacen(
                        "{}",
                        if total_lines == 1 {
                            ""
                        } else {
                            msgs.thinking_card_title_pl
                        },
                        1,
                    ),
            )
            .title_bottom(
                msgs.thinking_card_bottom
                    .replacen("{}", &(showing_from + 1).to_string(), 1)
                    .replacen("{}", &total_lines.to_string(), 1),
            );

        let card_area = Rect::new(
            area.x + 1,
            area.y + 1 + y_top,
            area.width.saturating_sub(2),
            y_bot - y_top,
        );
        frame.render_widget(card_block, card_area);

        let inner = Rect::new(
            card_area.x + 1,
            card_area.y + 1,
            card_area.width.saturating_sub(2),
            card_area.height.saturating_sub(2),
        );
        if inner.height > 0 && !block.cached_preview.is_empty() {
            let preview_style = Style::default()
                .fg(Color::Rgb(180, 180, 200))
                .bg(app.theme.bg);
            let start_preview = block.cached_preview.len().saturating_sub(3);
            let preview_lines: Vec<Line> = block.cached_preview[start_preview..]
                .iter()
                .take(3)
                .map(|s| {
                    let display = if s.len() > inner.width as usize {
                        format!("{}…", &s[..inner.width as usize - 1])
                    } else {
                        s.clone()
                    };
                    Line::from(Span::styled(display, preview_style))
                })
                .collect();
            frame.render_widget(Paragraph::new(preview_lines), inner);
        }
    }

    // ---- Diff block overlay ----
    for block in &app.diff_blocks {
        let Some(start_logical) = app.phys_to_logical_fast(block.start_idx) else {
            continue;
        };
        let Some(end_logical) = app.phys_to_logical_fast(block.end_idx) else {
            continue;
        };
        if start_logical >= vs_cache.len() || end_logical >= vs_cache.len() {
            continue;
        }
        let vis_top = vs_cache[start_logical];
        let vis_bot = vs_cache[end_logical];
        let vis_range_end = visual_scroll + visible_height;
        if vis_bot <= visual_scroll || vis_top >= vis_range_end {
            continue;
        }
        let y_top = (vis_top.saturating_sub(visual_scroll)) as u16;
        let y_bot = (vis_bot.saturating_sub(visual_scroll)).min(visible_height as _) as u16;
        if y_bot <= y_top {
            continue;
        }

        let content_lines: Vec<&str> = block.content.lines().collect();
        let total_lines = content_lines.len();

        let msgs = app.msgs();
        let card_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(app.theme.accent))
            .style(Style::default().bg(app.theme.bg))
            .title(
                msgs.diff_card_title
                    .replacen("{}", &total_lines.to_string(), 1)
                    .replacen("{}", &block.file_path, 1),
            )
            .title_bottom(Line::from(Span::styled(
                msgs.diff_card_bottom,
                Style::default().fg(app.theme.accent),
            )));

        let card_area = Rect::new(
            area.x + 1,
            area.y + 1 + y_top,
            area.width.saturating_sub(2),
            y_bot - y_top,
        );
        frame.render_widget(card_block, card_area);

        let inner = Rect::new(
            card_area.x + 1,
            card_area.y + 1,
            card_area.width.saturating_sub(2),
            card_area.height.saturating_sub(2),
        );
        if inner.height > 0 {
            let max_visible = inner.height as usize;
            let num_width = (total_lines + 1).to_string().len().max(3);
            let code_width = (inner.width as usize).saturating_sub(num_width + 3);
            let num_style = Style::default().fg(Color::Gray).bg(app.theme.bg);
            let text_style = Style::default().fg(app.theme.fg).bg(app.theme.bg);
            let plus_style = Style::default().fg(app.theme.success).bg(app.theme.bg);

            let mut preview_lines: Vec<Line> = content_lines
                .iter()
                .take(max_visible)
                .enumerate()
                .map(|(i, line)| {
                    let num = format!("{:>nw$}", i + 1, nw = num_width);
                    let trimmed: String = line.chars().take(code_width).collect();
                    Line::from(vec![
                        Span::styled(format!(" {} ", num), num_style),
                        Span::styled("+ ", plus_style),
                        Span::styled(trimmed, text_style),
                    ])
                })
                .collect();

            if total_lines > max_visible {
                preview_lines.push(Line::from(Span::styled(
                    app.msgs()
                        .diff_overflow_tmpl
                        .replace("{}", &(total_lines - max_visible).to_string()),
                    Style::default().fg(Color::Gray).bg(app.theme.bg),
                )));
            }

            frame.render_widget(Paragraph::new(preview_lines), inner);
        }
    }

    // Scrollbar
    let scrollbar = Scrollbar::default()
        .orientation(ratatui::widgets::ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"))
        .track_symbol(Some("│"))
        .thumb_symbol("█")
        .begin_style(Style::default().fg(app.theme.border))
        .end_style(Style::default().fg(app.theme.border))
        .track_style(Style::default().fg(app.theme.border))
        .thumb_style(Style::default().fg(app.theme.accent));
    let sb_position = if total_visual > visible_height {
        let range = total_visual - visible_height;
        (visual_scroll as u64 * (total_visual - 1) as u64 / range as u64) as usize
    } else {
        0
    };
    let sb_position = sb_position.min(total_visual.saturating_sub(1));
    let mut state = ScrollbarState::new(total_visual)
        .viewport_content_length(app.log_scroll.height as usize)
        .position(sb_position);
    frame.render_stateful_widget(scrollbar, area, &mut state);

    app.log_scroll.visual_start = app.log_scroll.visual_start_cache.clone();
}
