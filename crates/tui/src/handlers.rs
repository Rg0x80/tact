use crate::state::{App, FocusedPanel, InputMode, PALETTE_COMMANDS, Status};
use arboard::Clipboard;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Local;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::{ListState, ScrollbarState};
use tact_core::UserCommand;
use tokio::sync::mpsc::UnboundedSender;

/// 尝试将文本写入剪贴板。优先使用系统剪贴板，不可用时回退到 OSC 52
/// 终端序列，最后再保存到内部缓冲区。
fn copy_text(app: &mut App, text: &str) {
    let preview = text.chars().take(40).collect::<String>();

    // 1. 尝试原生剪贴板
    if let Ok(mut clip) = Clipboard::new() {
        if clip.set_text(text).is_ok() {
            let msgs = app.msgs();
            app.add_system_message(msgs.copied_tmpl.replace("{}", &preview));
            return;
        }
    }

    // 2. 回退：OSC 52 终端剪贴板（适用于 SSH / tmux 等场景）
    let encoded = BASE64.encode(text);
    let osc52 = format!("\x1b]52;c;{}\x07", encoded);
    if std::io::Write::write_all(&mut std::io::stdout(), osc52.as_bytes()).is_ok() {
        let msgs = app.msgs();
        app.add_system_message(msgs.copied_terminal_tmpl.replace("{}", &preview));
        return;
    }

    // 3. 最后手段：保存到内部缓冲区
    app.clipboard_buffer = text.to_string();
    let msgs = app.msgs();
    app.add_system_message(msgs.copied_internal_tmpl.replace("{}", &preview));
}

/// Normal 模式按键处理：面板切换、滚动、审批、搜索、命令面板、剪贴板等。
pub(super) fn handle_normal_mode(
    app: &mut App,
    key: KeyEvent,
    _user_cmd_tx: &UnboundedSender<UserCommand>,
) {
    match key.code {
        KeyCode::Tab => {
            app.focused_panel = match app.focused_panel {
                FocusedPanel::Log => FocusedPanel::Plan,
                FocusedPanel::Plan => FocusedPanel::Log,
            };
        }
        KeyCode::Char('e') => {
            app.plan.visible = !app.plan.visible;
            if !app.plan.visible && app.focused_panel == FocusedPanel::Plan {
                app.focused_panel = FocusedPanel::Log;
            }
        }
        KeyCode::Char('j') => match app.focused_panel {
            FocusedPanel::Log => {
                // 不检查上限，render 会统一 clamp
                app.log_scroll.offset = app.log_scroll.offset.saturating_add(1);
            }
            FocusedPanel::Plan => {
                if !app.plan.steps.is_empty() && app.plan.selected + 1 < app.plan.steps.len() {
                    app.plan.selected += 1;
                    app.plan.list_state.select(Some(app.plan.selected));
                }
            }
        },
        KeyCode::Char('k') => match app.focused_panel {
            FocusedPanel::Log => {
                if app.log_scroll.offset > 0 {
                    app.log_scroll.offset -= 1;
                }
            }
            FocusedPanel::Plan => {
                if app.plan.selected > 0 {
                    app.plan.selected -= 1;
                    app.plan.list_state.select(Some(app.plan.selected));
                }
            }
        },
        KeyCode::Char('g') => {
            if app.focused_panel == FocusedPanel::Log {
                app.log_scroll.offset = 0;
            }
        }
        KeyCode::Char('G') => {
            if app.focused_panel == FocusedPanel::Log {
                // 设为一个足够大的值，render 会 clamp 到实际 max_scroll
                app.log_scroll.offset = u16::MAX;
            }
        }
        KeyCode::Char('n') => {
            if matches!(&app.status, Status::WaitingForUser { .. }) {
                let old_status = std::mem::replace(&mut app.status, Status::Idle);
                if let Status::WaitingForUser { approval_tx, .. } = old_status {
                    let _ = approval_tx.send(false);
                    let msgs = app.msgs();
                    app.add_system_message(msgs.step_rejected.to_string());
                }
            } else {
                app.jump_to_next_match();
            }
        }
        KeyCode::Char('N') => {
            app.jump_to_prev_match();
        }
        KeyCode::Char('/') => {
            app.input_mode = InputMode::Search;
            app.cmd_line.clear();
        }
        KeyCode::Char(':') => {
            app.input_mode = InputMode::Palette;
            app.cmd_line.clear();
            app.palette_selected = 0;
        }
        KeyCode::Char('i') | KeyCode::Enter => {
            app.input_mode = InputMode::Insert;
        }
        KeyCode::Char('y') => {
            let old_status = std::mem::replace(&mut app.status, Status::Idle);
            if let Status::WaitingForUser {
                prompt: _,
                step_index: _,
                approval_tx,
            } = old_status
            {
                let _ = approval_tx.send(true);
                let msgs = app.msgs();
                app.add_system_message(msgs.step_approved.to_string());
                app.add_new_line();
            } else {
                // Copy to clipboard based on focused panel
                let text = match app.focused_panel {
                    FocusedPanel::Plan => {
                        if let Some((s, e)) = app.mouse.plan_selection {
                            let start = s.min(e);
                            let end = s.max(e);
                            if start < app.plan.steps.len() {
                                let selected: Vec<String> = app.plan.steps
                                    [start..=end.min(app.plan.steps.len().saturating_sub(1))]
                                    .iter()
                                    .map(|step| step.description.clone())
                                    .collect();
                                Some(selected.join("\n"))
                            } else {
                                None
                            }
                        } else {
                            app.plan
                                .steps
                                .get(app.plan.selected)
                                .map(|s| s.description.clone())
                        }
                    }
                    FocusedPanel::Log => {
                        // Prefer mouse selection over last message
                        if let Some((s, e)) = app.mouse.log_selection {
                            let start = s.min(e);
                            let end = s.max(e);
                            // 如果有单词选择（双击），复制单词而非整行
                            if let Some((word_start, word_end)) =
                                app.mouse.log_word_selection
                            {
                                if let Some(phys_idx) =
                                    app.visible_message_index(start)
                                {
                                    let text = &app.raw_messages[phys_idx];
                                    let word = &text[word_start.min(text.len())
                                        ..word_end.min(text.len())];
                                    Some(word.to_string())
                                } else {
                                    None
                                }
                            } else {
                                let mut selected = Vec::new();
                                for logical_i in start..=end {
                                    if let Some(phys_idx) =
                                        app.visible_message_index(logical_i)
                                    {
                                        selected.push(
                                            app.raw_messages[phys_idx].as_str(),
                                        );
                                    }
                                }
                                if selected.is_empty() {
                                    None
                                } else {
                                    Some(selected.join("\n"))
                                }
                            }
                        } else {
                            // 最后一个可见消息
                            let total = app.total_log_lines();
                            if total > 0 && app.stream.buffer.is_empty() {
                                app.visible_message_index(total - 1)
                                    .and_then(|idx| app.raw_messages.get(idx).cloned())
                            } else if !app.stream.buffer.is_empty() {
                                Some(app.stream.buffer.clone())
                            } else {
                                None
                            }
                        }
                    }
                };
                if let Some(t) = text {
                    copy_text(app, &t);
                    app.add_new_line();
                }
                // 恢复之前的 status。WaitingForUser 已在 y/n 分支上方处理，此处无需再次恢复。
                if !matches!(old_status, Status::WaitingForUser { .. }) {
                    app.status = old_status;
                }
            }
        }
        KeyCode::Char('Y') => {
            if app.focused_panel == FocusedPanel::Log {
                if let Some(code) = app.extract_last_code_block() {
                    copy_text(app, &code);
                    app.add_new_line();
                }
            }
        }
        KeyCode::Char('c') => {
            if !matches!(app.status, Status::Idle) {
                let _ = _user_cmd_tx.send(UserCommand::Cancel);
            }
        }
        KeyCode::Char('q') => {
            app.should_quit = true;
        }
        KeyCode::Esc => {
            app.mouse.log_selection = None;
            app.mouse.plan_selection = None;
        }
        _ => {}
    }
}

/// 返回指定字节位置前一个字符的起始字节索引。
fn prev_char_boundary(s: &str, cursor: usize) -> usize {
    s[..cursor]
        .char_indices()
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// 返回指定字节位置后一个字符的起始字节索引。
fn next_char_boundary(s: &str, cursor: usize) -> usize {
    s[cursor..]
        .chars()
        .next()
        .map(|c| cursor + c.len_utf8())
        .unwrap_or(cursor)
}

/// 返回光标位置所在行起始的字节索引。
fn start_of_line(s: &str, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    s[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

/// 返回光标位置所在行末尾的字节索引（即行尾换行符的字节位置；若为最后一行则是字符串长度）。
fn end_of_line(s: &str, cursor: usize) -> usize {
    s[cursor..].find('\n').map(|i| cursor + i).unwrap_or(s.len())
}

/// 退出历史导航模式。
fn exit_history(app: &mut App) {
    app.input_history.index = None;
    app.input_history.saved.clear();
}

/// 计算光标所在的 (行, 列)，列按字符计数。
fn cursor_line_col(s: &str, cursor: usize) -> (usize, usize) {
    let mut line = 0;
    let mut col = 0;
    for (i, c) in s.char_indices() {
        if i >= cursor {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// 返回指定行的字符长度（不含换行符）。
fn line_length(s: &str, target_line: usize) -> usize {
    let mut line = 0;
    let mut len = 0;
    for c in s.chars() {
        if line == target_line {
            if c == '\n' {
                break;
            }
            len += 1;
        } else if c == '\n' {
            line += 1;
        }
    }
    len
}

/// 将 (行, 列) 转换为字节索引。
fn line_col_to_cursor(s: &str, target_line: usize, target_col: usize) -> usize {
    let mut line = 0;
    let mut col = 0;
    for (i, c) in s.char_indices() {
        if line == target_line && col == target_col {
            return i;
        }
        if c == '\n' {
            if line == target_line {
                return i;
            }
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    s.len()
}

/// 判断字符是否为单词字符（字母、数字、下划线）。
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 返回光标位置前一个单词的起始字节索引（向后删除词）。
fn prev_word_boundary(s: &str, cursor: usize) -> usize {
    let mut pos = cursor;
    let mut chars = s[..cursor].chars().rev().peekable();

    // 跳过空白字符
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            pos -= c.len_utf8();
            chars.next();
        } else {
            break;
        }
    }

    // 记录第一个非空白字符的类型，并跳过同类字符
    if let Some(&first) = chars.peek() {
        if is_word_char(first) {
            while let Some(&c) = chars.peek() {
                if is_word_char(c) {
                    pos -= c.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if !c.is_whitespace() && !is_word_char(c) {
                    pos -= c.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
        }
    }

    pos
}

/// 返回光标位置后一个单词的结束字节索引（向前删除词）。
fn next_word_boundary(s: &str, cursor: usize) -> usize {
    let mut pos = cursor;
    let mut chars = s[cursor..].chars().peekable();

    // 跳过空白字符
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            pos += c.len_utf8();
            chars.next();
        } else {
            break;
        }
    }

    // 记录第一个非空白字符的类型，并跳过同类字符
    if let Some(&first) = chars.peek() {
        if is_word_char(first) {
            while let Some(&c) = chars.peek() {
                if is_word_char(c) {
                    pos += c.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if !c.is_whitespace() && !is_word_char(c) {
                    pos += c.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
        }
    }

    pos
}

/// Insert 模式按键处理：接收用户任务输入，Enter 提交给 Agent。
pub(super) fn handle_insert_mode(
    app: &mut App,
    key: KeyEvent,
    user_cmd_tx: &UnboundedSender<UserCommand>,
) {
    match key.code {
        KeyCode::Enter => {
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::SHIFT)
                || key.modifiers.contains(crossterm::event::KeyModifiers::ALT)
            {
                // insert blank charater for writing next line
                app.input.insert(app.input_cursor, '\n');
                app.input_cursor += 1;
            } else if !app.input.is_empty() {
                // Save to history (skip consecutive duplicates)
                let task_text = app.input.clone();
                if app.input_history.entries.last() != Some(&task_text) {
                    app.input_history.entries.push(task_text.clone());
                    if app.input_history.entries.len() > 100 {
                        app.input_history.entries.remove(0);
                    }
                    app.save_history();
                }
                app.input_history.index = None;
                app.input_history.saved.clear();
                let task = std::mem::take(&mut app.input);
                app.input_cursor = 0;
                // If waiting for approval, reject it before starting new task
                let old_status = std::mem::replace(&mut app.status, Status::Planning);
                if let Status::WaitingForUser { approval_tx, .. } = old_status {
                    let _ = approval_tx.send(false);
                    let msgs = app.msgs();
                    app.add_system_message(msgs.approval_cancelled.to_string());
                }
                let blank_task = format!("{}", task.clone());
                app.add_user_message(blank_task);
                app.plan.steps.clear();
                app.plan.collapsed.clear();
                app.plan.selected = 0;
                app.plan.list_state = ListState::default();
                app.plan.scroll_state = ScrollbarState::new(0);
                app.task_start_time = Some(chrono::Local::now());
                // 发送命令给agent
                let _ = user_cmd_tx.send(UserCommand::SubmitTask(task));
            }
        }
        // 快速删除单词
        KeyCode::Char('w')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            // Editing a history entry exits history navigation
            app.input_history.index = None;
            app.input_history.saved.clear();
            if app.input_cursor > 0 {
                let pos = prev_word_boundary(&app.input, app.input_cursor);
                app.input.drain(pos..app.input_cursor);
                app.input_cursor = pos;
            }
        }
        KeyCode::Backspace if key.modifiers.contains(crossterm::event::KeyModifiers::ALT) => {
            // Editing a history entry exits history navigation
            app.input_history.index = None;
            app.input_history.saved.clear();
            if app.input_cursor > 0 {
                let pos = prev_word_boundary(&app.input, app.input_cursor);
                app.input.drain(pos..app.input_cursor);
                app.input_cursor = pos;
            }
        }
        // Ctrl+A: 跳转到输入开头
        KeyCode::Char('a')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            app.input_cursor = 0;
        }
        // Ctrl+E: 跳转到输入末尾
        KeyCode::Char('e')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            app.input_cursor = app.input.len();
        }
        // Ctrl+K: 删除到行尾（kill-line），若光标在行尾则删除换行符以合并下行
        KeyCode::Char('k')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            exit_history(app);
            let end = end_of_line(&app.input, app.input_cursor);
            let delete_end = if end < app.input.len() { end + 1 } else { end };
            app.input.drain(app.input_cursor..delete_end);
        }
        KeyCode::Char('d') if key.modifiers.contains(crossterm::event::KeyModifiers::ALT) => {
            // Editing a history entry exits history navigation
            app.input_history.index = None;
            app.input_history.saved.clear();
            if app.input_cursor < app.input.len() {
                let pos = next_word_boundary(&app.input, app.input_cursor);
                app.input.drain(app.input_cursor..pos);
            }
        }
        // Ctrl+U: 删除到行首
        KeyCode::Char('u')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            exit_history(app);
            let start = start_of_line(&app.input, app.input_cursor);
            app.input.drain(start..app.input_cursor);
            app.input_cursor = start;
        }
        // Ctrl+D: 删除光标后一个字符（仅当输入非空时）
        KeyCode::Char('d')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            exit_history(app);
            if app.input_cursor < app.input.len() {
                let next = next_char_boundary(&app.input, app.input_cursor);
                app.input.drain(app.input_cursor..next);
            }
        }
        // Ctrl+Home: 跳转到输入开头
        KeyCode::Home
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            app.input_cursor = 0;
        }
        // Ctrl+End: 跳转到输入末尾
        KeyCode::End
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            app.input_cursor = app.input.len();
        }
        // Ctrl+Backspace: 删除前一个单词（与 Ctrl+W / Alt+Backspace 一致）
        KeyCode::Backspace
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            exit_history(app);
            let pos = prev_word_boundary(&app.input, app.input_cursor);
            app.input.drain(pos..app.input_cursor);
            app.input_cursor = pos;
        }
        KeyCode::Char(c) => {
            // Typing anything exits history navigation
            app.input_history.index = None;
            app.input_history.saved.clear();
            app.input.insert(app.input_cursor, c);
            app.input_cursor += c.len_utf8();
        }
        KeyCode::Backspace => {
            // Editing a history entry exits history navigation
            app.input_history.index = None;
            app.input_history.saved.clear();
            if app.input_cursor > 0 {
                let prev = prev_char_boundary(&app.input, app.input_cursor);
                app.input.remove(prev);
                app.input_cursor = prev;
            }
        }
        KeyCode::Delete => {
            // Editing a history entry exits history navigation
            app.input_history.index = None;
            app.input_history.saved.clear();
            if app.input_cursor < app.input.len() {
                app.input.remove(app.input_cursor);
            }
        }
        // 快速移动游标（按单词）
        KeyCode::Left
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
                || key.modifiers.contains(crossterm::event::KeyModifiers::ALT) =>
        {
            app.input_cursor = prev_word_boundary(&app.input, app.input_cursor);
        }
        KeyCode::Right
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
                || key.modifiers.contains(crossterm::event::KeyModifiers::ALT) =>
        {
            app.input_cursor = next_word_boundary(&app.input, app.input_cursor);
        }
        KeyCode::Left => {
            app.input_cursor = prev_char_boundary(&app.input, app.input_cursor);
        }
        KeyCode::Right => {
            app.input_cursor = next_char_boundary(&app.input, app.input_cursor);
        }
        KeyCode::Up => {
            let (line, _col) = cursor_line_col(&app.input, app.input_cursor);
            if line > 0 {
                // Multi-line: move cursor up within current input
                let new_col = _col.min(line_length(&app.input, line - 1));
                app.input_cursor = line_col_to_cursor(&app.input, line - 1, new_col);
            } else if !app.input_history.entries.is_empty() {
                // Cursor at first line → navigate history backward
                if app.input_history.index.is_none() {
                    // Enter history mode: save current input and start from the end
                    app.input_history.saved = app.input.clone();
                    app.input_history.index = Some(app.input_history.entries.len() - 1);
                } else if let Some(idx) = app.input_history.index {
                    if idx > 0 {
                        app.input_history.index = Some(idx - 1);
                    }
                }
                if let Some(idx) = app.input_history.index {
                    app.input = app.input_history.entries[idx].clone();
                    app.input_cursor = app.input.len();
                }
            }
        }
        KeyCode::Down => {
            let (line, _col) = cursor_line_col(&app.input, app.input_cursor);
            let next_len = line_length(&app.input, line + 1);
            if next_len > 0 || line_col_to_cursor(&app.input, line + 1, 0) < app.input.len() {
                // Multi-line: move cursor down within current input
                let new_col = _col.min(next_len);
                app.input_cursor = line_col_to_cursor(&app.input, line + 1, new_col);
            } else if app.input_history.index.is_some() {
                // Cursor at last line and in history mode → navigate forward
                if let Some(idx) = app.input_history.index {
                    if idx + 1 < app.input_history.entries.len() {
                        app.input_history.index = Some(idx + 1);
                        app.input = app.input_history.entries[idx + 1].clone();
                        app.input_cursor = app.input.len();
                    } else {
                        // Past the newest entry → restore saved input
                        app.input_history.index = None;
                        app.input = std::mem::take(&mut app.input_history.saved);
                        app.input_cursor = app.input.len();
                    }
                }
            }
        }
        KeyCode::Home => {
            let (line, _) = cursor_line_col(&app.input, app.input_cursor);
            app.input_cursor = line_col_to_cursor(&app.input, line, 0);
        }
        KeyCode::End => {
            let (line, _) = cursor_line_col(&app.input, app.input_cursor);
            app.input_cursor = line_col_to_cursor(&app.input, line, line_length(&app.input, line));
        }
        KeyCode::Esc => app.input_mode = InputMode::Normal,
        _ => {}
    }
}

/// Search 模式按键处理：输入搜索关键词，Enter 确认后高亮匹配内容。
pub(super) fn handle_search_mode(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            app.search.term = app.cmd_line.clone();
            app.update_search_matches();
            app.cmd_line.clear();
            app.input_mode = InputMode::Normal;
        }
        // Ctrl+W: 删除最后一个词
        KeyCode::Char('w')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            let pos = prev_word_boundary(&app.cmd_line, app.cmd_line.len());
            app.cmd_line.drain(pos..);
        }
        // Ctrl+U: 清空搜索输入
        KeyCode::Char('u')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            app.cmd_line.clear();
        }
        KeyCode::Char(c) => app.cmd_line.push(c),
        KeyCode::Backspace => {
            app.cmd_line.pop();
        }
        KeyCode::Esc => {
            app.cmd_line.clear();
            app.input_mode = InputMode::Normal;
        }
        _ => {}
    }
}

/// Palette 模式按键处理：过滤命令列表并通过上下键选择，Enter 执行。
pub(super) fn handle_palette_mode(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            let filter = app.cmd_line.to_lowercase();
            let filtered: Vec<usize> = PALETTE_COMMANDS
                .iter()
                .enumerate()
                .filter(|(_, (cmd, desc))| {
                    filter.is_empty()
                        || cmd.to_lowercase().contains(&filter)
                        || desc.to_lowercase().contains(&filter)
                })
                .map(|(i, _)| i)
                .collect();
            if !filtered.is_empty() {
                let idx = app.palette_selected.min(filtered.len() - 1);
                let cmd = PALETTE_COMMANDS[filtered[idx]].0;
                app.cmd_line.clear();
                app.input_mode = InputMode::Normal;
                execute_palette_command(app, cmd);
            }
        }
        // Ctrl+W: 删除最后一个词
        KeyCode::Char('w')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            let pos = prev_word_boundary(&app.cmd_line, app.cmd_line.len());
            app.cmd_line.drain(pos..);
            app.palette_selected = 0;
        }
        // Ctrl+U: 清空 palette 输入
        KeyCode::Char('u')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            app.cmd_line.clear();
            app.palette_selected = 0;
        }
        KeyCode::Char(c) => {
            app.cmd_line.push(c);
            app.palette_selected = 0;
        }
        KeyCode::Backspace => {
            app.cmd_line.pop();
            app.palette_selected = 0;
        }
        KeyCode::Up => {
            if app.palette_selected > 0 {
                app.palette_selected -= 1;
            }
        }
        KeyCode::Down => {
            app.palette_selected += 1;
        }
        KeyCode::Esc => {
            app.cmd_line.clear();
            app.input_mode = InputMode::Normal;
        }
        _ => {}
    }
}

/// Select 弹窗模式按键处理：上下移动选择，Enter 确认，Esc 取消。
pub(super) fn handle_select_mode(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            if app.select.options.is_empty() {
                let msgs = app.msgs();
                app.add_system_message(msgs.no_options.to_string());
            } else {
                let idx = app.select.confirm().unwrap_or(0);
                let msgs = app.msgs();
                app.add_system_message(
                    msgs.selected_tmpl
                        .replace("{}", &app.select
                            .options
                            .get(idx)
                            .cloned()
                            .unwrap_or_else(|| "?".to_string()))
                );
            }
            app.input_mode = InputMode::Normal;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.select.move_down();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.select.move_up();
        }
        KeyCode::Esc => {
            app.select.cancel();
            let msgs = app.msgs();
            app.add_system_message(msgs.selection_cancelled.to_string());
            app.input_mode = InputMode::Normal;
        }
        _ => {}
    }
}

/// 执行命令面板中选中的命令。
pub(super) fn execute_palette_command(app: &mut App, cmd: &str) {
    match cmd {
        "theme" => app.toggle_theme(),
        "save" => {
            let timestamp = Local::now().format("%Y%m%d_%H%M%S");
            let filename = format!("agent_log_{}.txt", timestamp);
            if let Ok(mut file) = std::fs::File::create(&filename) {
                use std::io::Write;
                for msg in &app.raw_messages {
                    writeln!(file, "{}", msg).ok();
                }
                let msgs = app.msgs();
                app.add_system_message(msgs.log_saved_tmpl.replace("{}", &filename));
            } else {
                let msgs = app.msgs();
                app.add_system_message(msgs.log_save_failed.to_string());
            }
        }
        "quit" => app.should_quit = true,
        "help" => {
            app.show_help = !app.show_help;
            app.show_history = false;
        }
        "history" => {
            app.show_history = !app.show_history;
            app.show_help = false;
        }
        "search" => {
            app.input_mode = InputMode::Search;
            app.cmd_line.clear();
        }
        "cancel" => {
            if !matches!(app.status, Status::Idle) {
                let _ = app.user_cmd_tx.send(UserCommand::Cancel);
            }
        }
        "balance" => {
            let _ = app.user_cmd_tx.send(UserCommand::QueryBalance);
        }
        "lang" => {
            app.toggle_language();
        }
        "party" => {
            app.toggle_party_mode();
        }
        _ => {}
    }
}
