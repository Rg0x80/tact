use crate::state::*;
use crate::render::render_md::{format_table, is_horizontal_rule, render_markdown_tui};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListState, ScrollbarState};
use tact_core::{AgentErrorKind, AgentUpdate, StepStatus, UserCommand};

const CODE_BG: Color = Color::Rgb(30, 35, 50);
const CODE_FG: Color = Color::Rgb(200, 200, 210);
const STREAMING_INDICATOR: &str = " ▌";

impl App {
    pub(crate) fn handle_agent_update(&mut self, update: AgentUpdate) {
        self.dirty = true;
        // 关闭上一个 thinking 块：任何非 ThinkingChunk 的更新到来时，
        // 意味着 LLM 已完成 thinking 阶段，后续输出不属 thinking 区域。
        if !matches!(update, AgentUpdate::ThinkingChunk(_)) {
            self.flush_and_close_thinking();
        }
        match update {
            AgentUpdate::PlanGenerated(plan) => {
                // 新任务开始，flush 残留流式行
                self.flush_stream_pending();

                let plan_len = plan.len();
                self.plan.steps = plan;
                self.plan.collapsed = vec![false; plan_len];
                self.plan.selected = 0;
                self.plan.list_state =
                    ListState::default().with_selected(if plan_len > 0 { Some(0) } else { None });
                self.status = Status::Executing {
                    current_step: 0,
                    total: plan_len,
                };
                let msgs = self.msgs();
                let plan_messages: Vec<String> = self
                    .plan
                    .steps
                    .iter()
                    .enumerate()
                    .map(|(i, step)| {
                        msgs.plan_step_tmpl
                            .replacen("{}", &(i + 1).to_string(), 1)
                            .replacen("{}", &step.description, 1)
                    })
                    .collect();
                self.add_system_message(format!(
                    "{}",
                    msgs.plan_generated_tmpl
                        .replace("{}", &plan_len.to_string())
                ));
                for msg in plan_messages {
                    self.add_system_message(msg);
                }
                self.plan.scroll_state = ScrollbarState::new(plan_len.saturating_sub(1));
            }
            AgentUpdate::StepStarted(idx) => {
                // 先 flush 残留的流式内容（特别是 thinking 最后一行），
                // 确保工具执行前的所有 LLM 输出已完整显示。
                self.flush_stream_pending();
                if let Status::Executing {
                    current_step,
                    total: _,
                } = &mut self.status
                {
                    *current_step = idx;
                }
                if let Some(step) = self.plan.steps.get(idx) {
                    let description = step.description.clone();
                    let msgs = self.msgs();
                    // 在内容和工具调用之间插入一行空行作为视觉分隔
                    self.add_system_message(msgs.step_started_tmpl.replace("{}", &description));
                }
            }
            AgentUpdate::StepFinished(idx, result) => {
                // 关闭当前步骤的 thinking 块，避免下一个步骤的
                // ThinkingChunk 因 thinking_title_added 仍为 true 而跳过标题添加
                self.flush_stream_pending();
                let msgs = self.msgs();
                let icon = match result.status {
                    StepStatus::Success => msgs.step_success_prefix,
                    StepStatus::Failed => msgs.step_fail_prefix,
                };
                let log_msg = if result.arg_summary.is_empty() {
                    msgs.step_finished_simple_tmpl
                        .replacen("{}", icon, 1)
                        .replacen("{}", &(idx + 1).to_string(), 1)
                        .replacen("{}", &result.tool, 1)
                } else {
                    msgs.step_finished_args_tmpl
                        .replacen("{}", icon, 1)
                        .replacen("{}", &(idx + 1).to_string(), 1)
                        .replacen("{}", &result.tool, 1)
                        .replacen("{}", &result.arg_summary, 1)
                };
                let bytes_str = match result.tool.as_str() {
                    "read_file" | "write_file" => result
                        .detail
                        .as_ref()
                        .map(|d| msgs.step_bytes_tmpl.replace("{}", &d.len().to_string()))
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                let duration_str = result.duration_ms.map_or(String::new(), |ms| {
                    if ms < 1000 {
                        msgs.step_ms_tmpl.replace("{}", &ms.to_string())
                    } else {
                        format!(
                            "{}",
                            msgs.step_sec_tmpl
                                .replace("{}", &format!("{:.1}", ms as f64 / 1000.0))
                        )
                    }
                });
                let log_msg = format!("{}{}{}", log_msg, bytes_str, duration_str);
                self.add_system_message(log_msg);

                // 文件写入操作：插入占位行，由 render_log_panel 的 overlay 渲染 diff 块
                if result.tool == "write_file"
                    && let Some(content) = result.detail
                {
                    let content_lines = content.lines().count();
                    let max_preview = 20usize;
                    let preview_count = content_lines.min(max_preview);
                    // 占位行：顶部标题 + 预览内容 + 超出提示(可选) + 底部边框 + 分隔空行
                    let more = if content_lines > max_preview { 1 } else { 0 };
                    let placeholder_count = 2 + preview_count + more + 1;
                    let start_idx = self.messages.len();
                    for _ in 0..placeholder_count {
                        self.messages.push(Line::from(""));
                        self.raw_messages.push(String::new());
                    }
                    let end_idx = self.messages.len();
                    self.diff_blocks.push(DiffBlock {
                        start_idx,
                        end_idx,
                        file_path: result.arg_summary.clone(),
                        content: content.clone(),
                    });
                    self.log_scroll.state =
                        ScrollbarState::new(self.total_log_lines().saturating_sub(1));
                    if self.input_mode == InputMode::Insert || self.input_mode == InputMode::Normal
                    {
                        self.log_scroll.offset = u16::MAX;
                    }
                    if !self.search.term.is_empty() {
                        self.update_search_matches();
                    }
                }

                // 把输出预览存入 plan step，供 Plan 面板查看
                if let Some(step) = self.plan.steps.get_mut(idx) {
                    step.output = Some(result.message);
                }
            }
            AgentUpdate::StepFailed(idx, error) => {
                self.flush_stream_pending();
                let msgs = self.msgs();
                self.add_system_message(
                    msgs.step_failed_tmpl
                        .replacen("{}", &(idx + 1).to_string(), 1)
                        .replacen("{}", &error, 1),
                );
                self.status = Status::Idle;
                self.task_start_time = None;
            }
            // 处理需要用户审批的情况
            AgentUpdate::NeedApproval(prompt, step_idx, tx) => {
                // 先关闭活跃的 thinking 块，防止授权消息被卷入折叠区
                self.flush_stream_pending();
                let prompt_clone = prompt.clone();
                self.status = Status::WaitingForUser {
                    prompt,
                    step_index: step_idx,
                    approval_tx: tx,
                };
                self.input_mode = InputMode::Normal;
                let msgs = self.msgs();
                self.add_system_message(msgs.need_approval_tmpl.replace("{}", &prompt_clone));
            }
            AgentUpdate::TaskComplete(summary) => {
                // 任务完成，flush 残留流式行
                self.flush_stream_pending();
                //self.add_new_line();
                // 不再把 summary 重复渲染到 messages（StreamChunk 已实时展示）。
                // summary 只保存到 task_history 供历史记录查看。
                if let Some(entry) = self.task_history.last_mut() {
                    entry.summary = summary;
                }
                self.status = Status::Done;
                self.task_start_time = None;
                self.task_done_time = Some(chrono::Local::now());
            }
            // 错误处理
            AgentUpdate::Error(kind) => {
                match kind {
                    AgentErrorKind::BalanceNotSupported => {
                        self.balance_info = None;
                        self.flash_msg = Some((
                            "Balance query not supported for this model".to_string(),
                            std::time::Instant::now(),
                        ));
                        self.dirty = true;
                    }
                    AgentErrorKind::BalanceQueryFailed(err) => {
                        self.balance_info = None;
                        self.flash_msg = Some((
                            format!("Balance query failed: {}", err),
                            std::time::Instant::now(),
                        ));
                        self.dirty = true;
                    }
                    AgentErrorKind::Other(msg) => {
                        // 致命错误：flush 残留流式行
                        self.flush_stream_pending();
                        let msgs = self.msgs();
                        self.add_system_message(msgs.error_tmpl.replace("{}", &msg));
                        self.status = Status::Idle;
                        self.task_start_time = None;
                    }
                }
            }
            // 更新令牌使用信息
            AgentUpdate::TokenUsage {
                prompt,
                completion,
                total,
                prompt_cache_hit_tokens,
                prompt_cache_miss_tokens,
            } => {
                self.status_bar.token_prompt = prompt;
                self.status_bar.token_completion = completion;
                self.status_bar.token_total = total;
                self.status_bar.token_cache_hit = prompt_cache_hit_tokens;
                self.status_bar.token_cache_miss = prompt_cache_miss_tokens;
            }
            // 更新余额信息
            AgentUpdate::Balance(info) => {
                self.balance_info = Some(info.clone());
            }
            // 更新模型信息
            AgentUpdate::ModelInfo(params) => {
                self.status_bar.model_name = params.model;
                self.status_bar.model_max_tokens = params.max_tokens;
                self.status_bar.model_thinking_budget = params.thinking_budget;
            }
            // 添加系统消息
            AgentUpdate::Info(msg) => {
                self.add_system_message(msg);
            }
            AgentUpdate::StepAdded(step) => {
                // flush 残留的流式文本，避免 LLM 输出夹在 StepAdded 和 StepStarted 之间
                self.flush_stream_pending();
                let idx = self.plan.steps.len();
                self.plan.steps.push(step.clone());
                self.plan.collapsed.push(false);
                self.status = Status::Executing {
                    current_step: idx,
                    total: self.plan.steps.len(),
                };
                self.add_new_line();
                self.add_system_message(format!("  {}. {}", idx + 1, step.description));
                self.plan.scroll_state =
                    ScrollbarState::new(self.plan.steps.len().saturating_sub(1));
            }
            AgentUpdate::RequestSelect {
                prompt,
                options,
                respond,
            } => {
                self.select.set(prompt, options, respond);
                self.input_mode = InputMode::Select;
            }
            AgentUpdate::ThinkingChunk(text) => {
                self.thinking.buffer.push_str(&text);
                let msgs = self.msgs();

                // 第一次收到 thinking 时添加标题行
                if !self.thinking.title_added {
                    let title_style = Style::default()
                        .fg(Color::Gray)
                        .add_modifier(Modifier::ITALIC)
                        .bg(Color::Rgb(35, 35, 45));
                    // 在标题前插入空白隔离行，折叠前就建立视觉分隔
                    self.messages.push(Line::from(""));
                    self.raw_messages.push(String::new());
                    let separator_idx = self.messages.len() - 1;

                    self.messages.push(Line::from(Span::styled(
                        msgs.thinking_title.to_string(),
                        title_style,
                    )));
                    self.raw_messages.push(msgs.thinking_title.to_string());
                    self.thinking.title_added = true;
                    self.thinking.active_start = Some(separator_idx);
                }

                // 行级缓冲：提取完整行实时显示
                let style = Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::ITALIC)
                    .bg(Color::Rgb(35, 35, 45));
                while let Some(idx) = self.thinking.buffer.find('\n') {
                    let line = self.thinking.buffer[..idx].to_string();
                    self.thinking.buffer = self.thinking.buffer[idx + 1..].to_string();
                    let text = if line.is_empty() {
                        String::new()
                    } else {
                        msgs.thinking_line_prefix.replace("{}", &line).to_string()
                    };
                    self.messages
                        .push(Line::from(Span::styled(text.clone(), style)));
                    self.raw_messages.push(text);
                    self.thinking.active_end = Some(self.messages.len() - 1);
                }

                self.log_scroll.state =
                    ScrollbarState::new(self.total_log_lines().saturating_sub(1));
                if !self.search.term.is_empty() {
                    self.update_search_matches();
                }
                // u16::MAX 会被 render_log_panel 按视觉行数正确裁剪
                self.log_scroll.offset = u16::MAX;
            }
            AgentUpdate::StreamChunk(text) => {
                // flush 残留的 thinking 行（没有尾换行符的最后一行）
                // 注意：thinking 块已在 handle_agent_update 入口通过 gate 关闭。
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
                }
                self.stream.buffer.push_str(&text);

                // 行级缓冲：代码块按完整单元累积，表格行按表累积，普通行按段落累积
                let mut completed = Vec::new();
                while let Some(idx) = self.stream.buffer.find('\n') {
                    let line = self.stream.buffer[..idx].to_string();
                    self.stream.buffer = self.stream.buffer[idx + 1..].to_string();

                    let trimmed = line.trim();
                    let is_code_fence = trimmed.starts_with("```");
                    let is_code_fence_close = trimmed == "```" && self.stream.code_block;

                    if is_code_fence_close {
                        // Completed: replace streaming placeholders with a sized blank region,
                        // then store a CodeBlock overlay for card rendering.
                        const MAX_CODE_PREVIEW: usize = 30;
                        let lang = std::mem::take(&mut self.stream.code_block_lang);
                        let lines = std::mem::take(&mut self.stream.code_block_buffer);

                        if let Some(start_idx) = self.stream.code_block_start_idx.take() {
                            let stream_end = start_idx + self.stream.code_block_line_count;

                            if !lines.is_empty() {
                                let code_text =
                                    format!("```{}\n{}\n```", lang, lines.join("\n"));
                                let (styled, _) = render_markdown_tui(&code_text);
                                let placeholder_count =
                                    styled.len().min(MAX_CODE_PREVIEW) + 2; // +2 for card border
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
                                    content: lines.join("\n"),
                                    styled,
                                });
                            } else {
                                self.messages.drain(start_idx..stream_end);
                                self.raw_messages.drain(start_idx..stream_end);
                            }
                        } else if !lines.is_empty() {
                            let code_text = format!("```{}\n{}\n```", lang, lines.join("\n"));
                            let (styled, raw) = render_markdown_tui(&code_text);
                            completed.extend(styled.into_iter().zip(raw));
                        }
                        self.stream.code_block = false;
                        self.stream.code_block_line_count = 0;
                    } else if self.stream.code_block {
                        // Streaming: update previous line (remove indicator), append new line with indicator
                        self.stream.code_block_buffer.push(line.clone());

                        let prev_idx = self.messages.len().saturating_sub(1);
                        if self.stream.code_block_line_count > 1 {
                            if let Some(prev_raw) = self.raw_messages.get_mut(prev_idx) {
                                if prev_raw.ends_with(STREAMING_INDICATOR) {
                                    let clean = prev_raw
                                        .trim_end_matches(STREAMING_INDICATOR)
                                        .to_string();
                                    *prev_raw = clean.clone();
                                    self.messages[prev_idx] = Line::from(vec![
                                        Span::styled("│ ", Style::default().fg(Color::DarkGray).bg(CODE_BG)),
                                        Span::styled(clean, Style::default().fg(CODE_FG).bg(CODE_BG)),
                                    ]);
                                }
                            }
                        }

                        let display_line = format!("{}{}", line, STREAMING_INDICATOR);
                        self.messages.push(Line::from(vec![
                            Span::styled("│ ", Style::default().fg(Color::DarkGray).bg(CODE_BG)),
                            Span::styled(display_line, Style::default().fg(CODE_FG).bg(CODE_BG)),
                        ]));
                        self.raw_messages.push(line);
                        self.stream.code_block_line_count += 1;
                    } else if is_code_fence {
                        // Open new code block: flush pending content first
                        if !self.stream.paragraph.is_empty() {
                            let paragraph = std::mem::take(&mut self.stream.paragraph);
                            let (styled, raw) = render_markdown_tui(&paragraph);
                            completed.extend(styled.into_iter().zip(raw));
                        }
                        if !self.stream.table_buffer.is_empty() {
                            let (styled, raw) =
                                format_table(&self.stream.table_buffer, &self.theme);
                            completed.extend(styled.into_iter().zip(raw));
                            self.stream.table_buffer.clear();
                        }

                        // Flush completed lines so start_idx is accurate
                        for (styled_line, raw_line) in completed.drain(..) {
                            self.messages.push(styled_line);
                            self.raw_messages.push(raw_line);
                        }

                        let lang =
                            trimmed.strip_prefix("```").unwrap_or("").trim().to_string();
                        self.stream.code_block = true;
                        self.stream.code_block_buffer.clear();
                        self.stream.code_block_lang = lang.clone();
                        self.stream.code_block_start_idx = Some(self.messages.len());
                        self.stream.code_block_line_count = 1;

                        // Container header: ╭─ lang ─────
                        let label = if lang.is_empty() {
                            "code".to_string()
                        } else {
                            lang.clone()
                        };
                        let header_text = format!("╭─ {} ", label);
                        self.messages.push(Line::from(Span::styled(
                            header_text.clone(),
                            Style::default().fg(Color::DarkGray).bg(CODE_BG),
                        )));
                        self.raw_messages.push(format!("```{}", lang));
                    } else {
                        // 常规行处理
                        let is_table_line = trimmed.starts_with('|');
                        let is_blank = trimmed.is_empty();
                        let is_hr = is_horizontal_rule(&line);

                        if is_table_line {
                            if !self.stream.paragraph.is_empty() {
                                let paragraph = std::mem::take(&mut self.stream.paragraph);
                                let (styled, raw) = render_markdown_tui(&paragraph);
                                completed.extend(styled.into_iter().zip(raw));
                            }
                            self.stream.table_buffer.push(line);
                        } else if is_blank || is_hr {
                            if !self.stream.paragraph.is_empty() {
                                let paragraph = std::mem::take(&mut self.stream.paragraph);
                                let (styled, raw) = render_markdown_tui(&paragraph);
                                completed.extend(styled.into_iter().zip(raw));
                            }
                            if !self.stream.table_buffer.is_empty() {
                                let (styled, raw) =
                                    format_table(&self.stream.table_buffer, &self.theme);
                                completed.extend(styled.into_iter().zip(raw));
                                self.stream.table_buffer.clear();
                            }
                            if is_hr {
                                // 水平分隔线直接丢弃
                            } else {
                                completed.push((Line::from(""), String::new()));
                            }
                        } else {
                            if !self.stream.table_buffer.is_empty() {
                                let (styled, raw) =
                                    format_table(&self.stream.table_buffer, &self.theme);
                                completed.extend(styled.into_iter().zip(raw));
                                self.stream.table_buffer.clear();
                            }
                            if !self.stream.paragraph.is_empty() {
                                self.stream.paragraph.push('\n');
                            }
                            self.stream.paragraph.push_str(&line);
                        }
                    }
                }

                for (styled_line, raw_line) in completed {
                    self.messages.push(styled_line);
                    self.raw_messages.push(raw_line);
                }

                self.log_scroll.state =
                    ScrollbarState::new(self.total_log_lines().saturating_sub(1));
                if !self.search.term.is_empty() {
                    self.update_search_matches();
                }
                // 自动滚动到底部（u16::MAX 会被 render_log_panel 按视觉行数裁剪）
                self.log_scroll.offset = u16::MAX;
            }
        }
        // 尾部统一刷新 scroll 状态，兜底 flush_and_close_thinking / flush_stream_pending
        // 等 helper 插入消息后未更新 scroll 的情况（大部分 arm 会自行调 add_system_message，
        // StreamChunk / ThinkingChunk 也各自更新，此处的冗余调用开销极小且无害）。
        self.log_scroll.state = ScrollbarState::new(self.total_log_lines().saturating_sub(1));
        if !self.search.term.is_empty() {
            self.update_search_matches();
        }
    }


}
