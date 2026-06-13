use crate::state::{App, InputMode};
use crossterm::event::{KeyCode, KeyEvent};
use super::prev_word_boundary;

/// Search 模式按键处理：输入搜索关键词，Enter 确认后高亮匹配内容。
pub(crate) fn handle_search_mode(app: &mut App, key: KeyEvent) {
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
