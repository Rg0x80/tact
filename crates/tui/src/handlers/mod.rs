// Input handlers — split by mode.
mod normal;
mod insert;
mod search;
mod palette;
mod select;

pub(crate) use normal::handle_normal_mode;
pub(crate) use insert::handle_insert_mode;
pub(crate) use search::handle_search_mode;
pub(crate) use palette::handle_palette_mode;
pub(crate) use select::handle_select_mode;

use crate::state::{App, FocusedPanel, InputMode, PALETTE_COMMANDS, Status};
use arboard::Clipboard;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Local;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::{ListState, ScrollbarState};
use tact_core::UserCommand;
use tokio::sync::mpsc::UnboundedSender;


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
