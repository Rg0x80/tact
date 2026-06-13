use crate::state::*;
use crate::render::render_md::{format_table, render_markdown_tui};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListState, ScrollbarState};
use tact_core::{AgentUpdate, StepStatus};

const CODE_BG: Color = Color::Rgb(30, 35, 50);
const STREAMING_INDICATOR: &str = " ▌";

impl App {
    pub(crate) fn is_message_visible(&self, idx: usize) -> bool {
        for block in &self.thinking.blocks {
            if idx > block.title_idx && idx <= block.end_idx {
                let total = block.end_idx.saturating_sub(block.title_idx);
                let visible_start = block.scroll_offset.min(total.saturating_sub(1));
                let visible_end = (block.scroll_offset + 3).min(total);
                let relative = idx.saturating_sub(block.title_idx + 1);
                return relative >= visible_start && relative < visible_end;
            }
        }
        true
    }

    /// 将逻辑行号映射到 messages 的物理索引。
    /// 如果逻辑行号超出固定消息范围，返回 None（表示是未完成的流式行）。
    pub(crate) fn visible_message_index(&self, logical_idx: usize) -> Option<usize> {
        let mut visible_count = 0;
        for idx in 0..self.messages.len() {
            if self.is_message_visible(idx) {
                if visible_count == logical_idx {
                    return Some(idx);
                }
                visible_count += 1;
            }
        }
        None
    }

    /// 在指定逻辑行的原始文本中找到鼠标所在列的单词边界。
    /// 返回 (word_start_byte, word_end_byte)。
    /// 单词由字母、数字、下划线、连字符组成，其他字符为分隔符。
    pub(crate) fn find_word_bounds(
        &self,
        logical_idx: usize,
        col: usize,
    ) -> Option<(usize, usize)> {
        let phys_idx = self.visible_message_index(logical_idx)?;
        let text = self.raw_messages.get(phys_idx)?;
        let bytes = text.as_bytes();
        let mut byte_pos = 0;
        let mut char_count = 0;
        // 将列位置转换为字节偏移
        while byte_pos < bytes.len() && char_count < col {
            let c = text[byte_pos..].chars().next()?;
            byte_pos += c.len_utf8();
            char_count += 1;
        }
        if byte_pos >= bytes.len() || bytes.is_empty() {
            return None;
        }
        // 从点击位置向两侧扩展找到单词边界
        let classify = |b: u8| -> bool { b.is_ascii_alphanumeric() || b == b'_' || b == b'-' };
        let mut start = byte_pos;
        let mut end = byte_pos;
        // 向左扩展
        while start > 0 {
            if classify(bytes[start - 1]) {
                start -= 1;
            } else {
                break;
            }
        }
        // 向右扩展
        while end < bytes.len() {
            if classify(bytes[end]) {
                end += 1;
            } else {
                break;
            }
        }
        if start < end {
            Some((start, end))
        } else {
            None
        }
    }

    /// O(1) 版本：使用 render_log_panel 构建的缓存映射。
    /// 返回 None 表示该物理索引不可见或缓存尚未构建。
    pub(crate) fn phys_to_logical_fast(&self, phys_idx: usize) -> Option<usize> {
        self.log_scroll
            .phys_to_logical_cache
            .get(phys_idx)
            .copied()
            .flatten()
    }

    /// 将视觉行号（鼠标点击行）映射回逻辑行号。
    /// 依赖 render_log_panel 每帧更新的 log_scroll.visual_start 前缀数组。
    pub(crate) fn logical_from_visual(&self, visual_row: usize) -> usize {
        if self.log_scroll.visual_start.is_empty() {
            return visual_row;
        }
        match self.log_scroll.visual_start.binary_search(&visual_row) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        }
    }

    /// 当前 Log 区域的总逻辑行数（固定消息 + 未完成的流式行）。
    pub(crate) fn total_log_lines(&self) -> usize {
        let visible_count = (0..self.messages.len())
            .filter(|&idx| self.is_message_visible(idx))
            .count();
        visible_count + if self.stream.buffer.is_empty() { 0 } else { 1 }
    }

    /// 关闭当前活跃的 thinking 块，将其加入 thinking_blocks 并默认只展示最后 3 行。
    pub(crate) fn close_active_thinking_block(&mut self) {
        if let Some(blank_idx) = self.thinking.active_start.take() {
            let end = self.thinking.active_end.unwrap_or(blank_idx);
            self.thinking.active_end = None;
            self.thinking.title_added = false;
            // blank_idx 是标题上方的隔离空行（已在 ThinkingChunk 中插入）
            // 标题在 blank_idx+1，thinking 内容行在 blank_idx+2..=end
            if end > blank_idx {
                // 在末尾插入空行作为视觉分隔（上方的隔离行已在 streaming 时插入）
                self.messages.insert(end + 1, Line::from(""));
                self.raw_messages.insert(end + 1, String::new());

                let title_idx = blank_idx + 1;
                let end_idx = end; // 未受插入影响，因为 insert 在 end 之后
                let total = end_idx.saturating_sub(title_idx);
                let scroll_offset = if total > 3 { total - 3 } else { 0 };

                // 预渲染 Markdown 并缓存预览文本，避免弹窗/卡片每帧重复渲染
                let mut preview_lines = Vec::with_capacity(total);
                let mut raw_content = String::new();
                for i in 1..=total {
                    let phys_idx = title_idx + i;
                    if phys_idx < self.raw_messages.len() {
                        let line = &self.raw_messages[phys_idx];
                        let stripped = line.strip_prefix("│ ").unwrap_or(line);
                        preview_lines.push(stripped.to_string());
                        raw_content.push_str(stripped);
                        raw_content.push('\n');
                    }
                }
                let (styled_lines, _) = render_markdown_tui(&raw_content);

                self.thinking.blocks.push(ThinkingBlock {
                    title_idx,
                    end_idx,
                    scroll_offset,
                    cached_preview: preview_lines,
                    cached_markdown: styled_lines,
                });
            }
        }
        // log_scroll 裁剪下沉到 render_log_panel 中执行，
        // 避免在 update 阶段裁剪造成与当前屏幕渲染不一致的焦点偏移。
        // 见 render.rs 中 render_log_panel 开头的 clamp 逻辑。
    }

    /// Flush thinking buffer 中残留的行并关闭当前活跃的 thinking 块。
    /// 若没有活跃的 thinking 块则什么都不做。
    pub(crate) fn flush_and_close_thinking(&mut self) {
        if self.thinking.active_start.is_some() {
            if !self.thinking.buffer.is_empty() {
                let style = Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::ITALIC)
                    .bg(Color::Rgb(35, 35, 45));
                let flush_text = if self.thinking.buffer.trim().is_empty() {
                    String::new()
                } else {
                    format!("│ {}", self.thinking.buffer)
                };
                if !flush_text.is_empty() {
                    self.messages
                        .push(Line::from(Span::styled(flush_text.clone(), style)));
                    self.raw_messages.push(flush_text);
                }
                self.thinking.buffer.clear();
                self.thinking.active_end = Some(self.messages.len() - 1);
            }
            self.close_active_thinking_block();
        }
    }

    /// 将流式缓冲区中残留的内容 flush 到消息列表。
    pub(crate) fn flush_stream_pending(&mut self) {
        // flush 累积的表格
        if !self.stream.table_buffer.is_empty() {
            let (lines, raw_lines) = format_table(&self.stream.table_buffer, &self.theme);
            self.messages.extend(lines);
            self.raw_messages.extend(raw_lines);
            self.stream.table_buffer.clear();
        }
        // flush incomplete code block (interrupted stream)
        if self.stream.code_block {
            const MAX_CODE_PREVIEW: usize = 30;
            let lang = std::mem::take(&mut self.stream.code_block_lang);
            let code_lines = std::mem::take(&mut self.stream.code_block_buffer);

            if let Some(start_idx) = self.stream.code_block_start_idx.take() {
                let stream_end = start_idx + self.stream.code_block_line_count;
                if !code_lines.is_empty() {
                    let code_text = format!("```{}\n{}\n```", lang, code_lines.join("\n"));
                    let (styled, _) = render_markdown_tui(&code_text);
                    let placeholder_count = styled.len().min(MAX_CODE_PREVIEW) + 2;
                    let placeholders: Vec<Line<'static>> =
                        (0..placeholder_count).map(|_| Line::from("")).collect();
                    let raw_placeholders: Vec<String> =
                        (0..placeholder_count).map(|_| String::new()).collect();
                    let _: Vec<_> = self
                        .messages
                        .splice(start_idx..stream_end, placeholders)
                        .collect();
                    let _: Vec<_> = self
                        .raw_messages
                        .splice(start_idx..stream_end, raw_placeholders)
                        .collect();
                    self.code_blocks.push(CodeBlock {
                        start_idx,
                        end_idx: start_idx + placeholder_count,
                        lang,
                        content: code_lines.join("\n"),
                        styled,
                    });
                } else {
                    self.messages.drain(start_idx..stream_end);
                    self.raw_messages.drain(start_idx..stream_end);
                }
            } else if !code_lines.is_empty() {
                let code_text = format!("```{}\n{}\n```", lang, code_lines.join("\n"));
                let (lines, raw_lines) = render_markdown_tui(&code_text);
                self.messages.extend(lines);
                self.raw_messages.extend(raw_lines);
            }
            self.stream.code_block = false;
            self.stream.code_block_line_count = 0;
        }
        // flush 累积的段落（尚未遇到空行的内容，如流结束时的最后一段）
        if !self.stream.paragraph.is_empty() {
            let paragraph = std::mem::take(&mut self.stream.paragraph);
            let (lines, raw_lines) = render_markdown_tui(&paragraph);
            self.messages.extend(lines);
            self.raw_messages.extend(raw_lines);
        }
        // flush 残留的 thinking 行并关闭 thinking 块
        if !self.thinking.buffer.is_empty() {
            let style = Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::ITALIC)
                .bg(Color::Rgb(35, 35, 45));
            let text = if self.thinking.buffer.trim().is_empty() {
                String::new()
            } else {
                format!("│ {}", self.thinking.buffer)
            };
            if !text.is_empty() {
                self.messages
                    .push(Line::from(Span::styled(text.clone(), style)));
                self.raw_messages.push(text);
            }
            self.thinking.buffer.clear();
            self.thinking.active_end = Some(self.messages.len() - 1);
        }
        self.close_active_thinking_block();
        if self.stream.buffer.is_empty() {
            return;
        }
        let display = self.stream.buffer.clone();
        let (lines, raw_lines) = render_markdown_tui(&display);
        self.messages.extend(lines);
        self.raw_messages.extend(raw_lines);
        self.stream.buffer.clear();
    }


}
