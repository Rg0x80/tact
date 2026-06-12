use crate::render::util::wrap_line;
use crate::state::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarState},
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
    super::cells::thinking::render_thinking_cards(
        frame, area, app, visual_scroll, visible_height);

    // ---- Diff block overlay ----
    super::cells::diff::render_diff_cards(
        frame, area, app, visual_scroll, visible_height);

    // ---- Code block overlay ----
    super::cells::code::render_code_cards(
        frame, area, app, visual_scroll, visible_height);

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
