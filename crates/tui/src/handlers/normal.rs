use crate::state::{App, FocusedPanel, InputMode, Status};
use crossterm::event::{KeyCode, KeyEvent};
use tact_core::UserCommand;
use tokio::sync::mpsc::UnboundedSender;
use super::copy_text;

pub(crate) fn handle_normal_mode(
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
        KeyCode::Enter => {
            if matches!(&app.status, Status::WaitingForUser { .. }) {
                let old_status = std::mem::replace(&mut app.status, Status::Idle);
                if let Status::WaitingForUser { approval_tx, .. } = old_status {
                    let _ = approval_tx.send(true);
                    let msgs = app.msgs();
                    app.add_system_message(msgs.step_approved.to_string());
                    app.add_new_line();
                }
            } else {
                app.input_mode = InputMode::Insert;
            }
        }
        KeyCode::Char('i') => {
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
        KeyCode::Char('V') => {
            // 打开最可见的代码块弹窗
            if app.code_popup.is_some() {
                app.close_code_popup();
            } else if !app.code_blocks.is_empty() && app.focused_panel == FocusedPanel::Log {
                let logical_offset = app.log_scroll.offset as usize;
                // 找到 start_idx 最接近当前滚动位置（且不超过）的代码块
                let best = app
                    .code_blocks
                    .iter()
                    .enumerate()
                    .filter(|(_, block)| {
                        app.phys_to_logical_fast(block.start_idx)
                            .map(|l| l <= logical_offset)
                            .unwrap_or(false)
                    })
                    .last()
                    .or_else(|| app.code_blocks.iter().enumerate().last());
                if let Some((idx, _)) = best {
                    app.open_code_popup(idx);
                }
            }
        }
        KeyCode::Char('c') => {
            if !matches!(app.status, Status::Idle) {
                let _ = _user_cmd_tx.send(UserCommand::Cancel);
            }
        }
        KeyCode::Char('t') => {
            // 打开最近可见的 thinking 卡片弹窗
            if app.thinking.popup.is_some() {
                app.close_thinking_popup();
            } else if !app.thinking.blocks.is_empty() {
                // 找到 title_idx 最接近当前滚动位置（且不超过）的块，否则取最新
                let logical_offset = app.log_scroll.offset as usize;
                let best = app
                    .thinking
                    .blocks
                    .iter()
                    .filter(|b| {
                        app.phys_to_logical_fast(b.title_idx)
                            .map(|l| l <= logical_offset)
                            .unwrap_or(false)
                    })
                    .last()
                    .or_else(|| app.thinking.blocks.last());
                if let Some(block) = best {
                    let title_idx = block.title_idx;
                    app.open_thinking_popup(title_idx);
                }
            }
        }
        KeyCode::Char('q') => {
            app.should_quit = true;
        }
        KeyCode::Esc => {
            if matches!(&app.status, Status::WaitingForUser { .. }) {
                let old_status = std::mem::replace(&mut app.status, Status::Idle);
                if let Status::WaitingForUser { approval_tx, .. } = old_status {
                    let _ = approval_tx.send(false);
                    let msgs = app.msgs();
                    app.add_system_message(msgs.step_rejected.to_string());
                }
            } else {
                app.mouse.log_selection = None;
                app.mouse.plan_selection = None;
            }
        }
        _ => {}
    }
}
