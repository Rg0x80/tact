use crate::state::*;
use arboard::Clipboard;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Local;
use crate::render::render_md::render_markdown_tui;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListState, ScrollbarState};
use tact_core::UserCommand;

const CODE_BG: Color = Color::Rgb(30, 35, 50);
const STREAMING_INDICATOR: &str = " ▌";

impl App {
    pub(crate) fn retry_task(&mut self, task: String) {
        self.add_user_message(task.clone());
        self.plan.steps.clear();
        self.plan.collapsed.clear();
        self.plan.selected = 0;
        self.plan.list_state = ListState::default();
        self.plan.scroll_state = ScrollbarState::new(0);
        self.status = Status::Planning;
        let _ = self.user_cmd_tx.send(UserCommand::SubmitTask(task));
        self.show_history = false;
    }

    // 添加一个空行作为分隔符，用于在日志中区分不同的输入/输出块。
    pub(crate) fn add_new_line(&mut self) {
        self.messages.push(Line::from(""));
        self.raw_messages.push(String::new());
    }

    /// 打开 thinking 弹窗，根据点击的 thinking 块标题行索引来定位块。
    pub(crate) fn open_thinking_popup(&mut self, title_idx: usize) {
        if let Some((bi, block)) = self
            .thinking
            .blocks
            .iter()
            .enumerate()
            .find(|(_, b)| b.title_idx == title_idx)
        {
            let title = self.raw_messages[block.title_idx].clone();
            self.thinking.popup = Some(ThinkingPopup {
                block_idx: bi,
                title,
                scroll: 0,
            });
        }
    }

    /// 关闭 thinking 弹窗。
    pub(crate) fn close_thinking_popup(&mut self) {
        self.thinking.popup = None;
    }

    /// 弹窗内向上滚动。
    pub(crate) fn thinking_popup_scroll_up(&mut self) {
        if let Some(ref mut popup) = self.thinking.popup {
            popup.scroll = popup.scroll.saturating_sub(1);
        }
    }

    /// 弹窗内向下滚动（上限由渲染时的实际行数限制）。
    pub(crate) fn thinking_popup_scroll_down(&mut self) {
        if let Some(ref mut popup) = self.thinking.popup {
            popup.scroll = popup.scroll.saturating_add(1);
        }
    }

    /// 查找包含指定逻辑行号的代码块（返回逻辑行号范围，含首尾 ``` 标记行）。
    pub(crate) fn find_code_block_containing_logical(
        &self,
        target_logical: usize,
    ) -> Option<(usize, usize)> {
        let mut logical = 0;
        let mut block_start: Option<usize> = None;
        for phys_idx in 0..self.raw_messages.len() {
            if !self.is_message_visible(phys_idx) {
                continue;
            }
            let raw = &self.raw_messages[phys_idx];
            let trimmed = raw.trim();
            if trimmed.starts_with("```") {
                if block_start.is_none() {
                    block_start = Some(logical);
                } else if trimmed == "```" {
                    let start = block_start.unwrap();
                    let end = logical;
                    if target_logical >= start && target_logical <= end {
                        return Some((start, end));
                    }
                    block_start = None;
                }
            }
            logical += 1;
        }
        None
    }

    /// 从 raw_messages 中查找最后一个完整代码块的内容（不含 ``` 标记）。
    /// 返回 None 表示没有找到闭合的代码块。
    pub(crate) fn extract_last_code_block(&self) -> Option<String> {
        let raw = &self.raw_messages;
        // 从末尾向前查找闭合的 ```
        let mut end = raw.len();
        loop {
            if end == 0 {
                return None;
            }
            end -= 1;
            if raw[end].trim() == "```" {
                break;
            }
        }
        // 从闭合 ``` 之前向前查找开头的 ```lang
        let mut start = end;
        loop {
            if start == 0 {
                return None;
            }
            start -= 1;
            if raw[start].trim_start().starts_with("```") {
                // 提取内容行（不含首尾 ``` 标记）
                let content: Vec<&str> = raw[start + 1..end].iter().map(|s| s.as_str()).collect();
                return if content.is_empty() {
                    None
                } else {
                    Some(content.join("\n"))
                };
            }
        }
    }

    /// 复制当前 thinking 弹窗的完整内容到剪贴板。
    pub(crate) fn copy_thinking_popup(&mut self) {
        let popup = match &self.thinking.popup {
            Some(p) => p,
            None => return,
        };
        let block = &self.thinking.blocks[popup.block_idx];
        if block.cached_preview.is_empty() {
            return;
        }
        let text = block.cached_preview.join("\n");
        let preview = if text.chars().count() > 40 {
            format!("{}…", text.chars().take(40).collect::<String>())
        } else {
            text.clone()
        };

        // 1. 尝试原生剪贴板
        if let Ok(mut clip) = Clipboard::new()
            && clip.set_text(&text).is_ok()
        {
            self.add_system_message(format!("📋 Copied: {}", preview));
            return;
        }

        // 2. 回退：OSC 52 终端剪贴板
        let encoded = BASE64.encode(&text);
        let osc52 = format!("\x1b]52;c;{}\x07", encoded);
        if std::io::Write::write_all(&mut std::io::stdout(), osc52.as_bytes()).is_ok() {
            self.add_system_message(format!("📋 Copied to terminal clipboard: {}", preview));
            return;
        }

        // 3. 最后手段：保存到内部缓冲区
        self.clipboard_buffer = text;
        self.add_system_message(format!(
            "📋 Copied to internal buffer (clipboard unavailable): {}",
            preview
        ));
        self.thinking.popup = None;
    }

    /// 打开文件内容弹窗，接受 diff 块首行索引。
    pub(crate) fn open_diff_popup(&mut self, start_idx: usize) {
        if let Some((bi, block)) = self
            .diff_blocks
            .iter()
            .enumerate()
            .find(|(_, b)| b.start_idx == start_idx)
        {
            self.diff_popup = Some(DiffPopup {
                block_idx: bi,
                file_path: block.file_path.clone(),
                content: block.content.clone(),
                scroll: 0,
            });
        }
    }

    /// 关闭文件内容弹窗。
    pub(crate) fn close_diff_popup(&mut self) {
        self.diff_popup = None;
    }

    /// 弹窗内向上滚动。
    pub(crate) fn diff_popup_scroll_up(&mut self) {
        if let Some(ref mut popup) = self.diff_popup {
            popup.scroll = popup.scroll.saturating_sub(1);
        }
    }

    /// 弹窗内向下滚动（上限由渲染时的实际行数限制）。
    pub(crate) fn diff_popup_scroll_down(&mut self) {
        if let Some(ref mut popup) = self.diff_popup {
            popup.scroll = popup.scroll.saturating_add(1);
        }
    }

    /// 复制弹窗文件内容到剪贴板。
    pub(crate) fn copy_diff_popup(&mut self) {
        let popup = match &self.diff_popup {
            Some(p) => p,
            None => return,
        };
        let text = &popup.content;
        let preview = if text.chars().count() > 40 {
            format!("{}…", text.chars().take(40).collect::<String>())
        } else {
            text.clone()
        };

        if let Ok(mut clip) = Clipboard::new()
            && clip.set_text(text).is_ok()
        {
            self.add_system_message(format!("📋 Copied: {}", preview));
            return;
        }
        let encoded = BASE64.encode(text);
        let osc52 = format!("\x1b]52;c;{}\x07", encoded);
        if std::io::Write::write_all(&mut std::io::stdout(), osc52.as_bytes()).is_ok() {
            self.add_system_message(format!("📋 Copied to terminal clipboard: {}", preview));
            return;
        }
        self.clipboard_buffer = text.clone();
        self.add_system_message(format!(
            "📋 Copied to internal buffer (clipboard unavailable): {}",
            preview
        ));
        self.diff_popup = None;
    }
}

