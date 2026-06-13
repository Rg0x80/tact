use crate::state::{App, InputMode, PALETTE_COMMANDS};
use crossterm::event::{KeyCode, KeyEvent};
use super::{execute_palette_command, prev_word_boundary};

/// Palette 模式按键处理：过滤命令列表并通过上下键选择，Enter 执行。
pub(crate) fn handle_palette_mode(app: &mut App, key: KeyEvent) {
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
