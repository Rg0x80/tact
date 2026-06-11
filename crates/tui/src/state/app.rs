// impl App — core application logic
// Extracted from state.rs to keep file sizes manageable.

use super::{
    App, DiffBlock, DiffPopup, FocusedPanel, HistoryEntry, InputHistory, InputMode, LogScroll,
    MouseState, PlanPanel, SearchState, SelectPopup, Status, StatusBarState, StreamState,
    ThinkingBlock, ThinkingPopup, ThinkingState,
    render_md::{format_table, is_horizontal_rule, render_markdown_tui},
};
use crate::i18n::{Language, Messages};
use crate::theme::{Theme, ThemeName};
use arboard::Clipboard;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Local;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListState, ScrollbarState};
use std::path::{Path, PathBuf};
use tact_core::{AgentErrorKind, AgentUpdate, StepStatus, UserCommand};

const CODE_BG: Color = Color::Rgb(30, 35, 50);
const CODE_FG: Color = Color::Rgb(200, 200, 210);
const STREAMING_INDICATOR: &str = " ▌";
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

impl App {
    /// 创建初始化的 App 实例，默认进入 Insert 模式并使用 Nord 主题。
    pub(crate) fn new(
        agent_rx: UnboundedReceiver<AgentUpdate>,
        user_cmd_tx: UnboundedSender<UserCommand>,
        work_dir: PathBuf,
    ) -> Self {
        let git_branch = std::process::Command::new("git")
            .args(["branch", "--show-current"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let workspace_dir = {
            let cwd = std::env::current_dir().ok();
            let home = std::env::var("HOME").ok();
            match (cwd, home) {
                (Some(p), Some(h)) => {
                    let path = p.to_string_lossy().to_string();
                    if path.starts_with(&h) {
                        format!("~{}", &path[h.len()..])
                    } else {
                        path
                    }
                }
                (Some(p), None) => p.to_string_lossy().to_string(),
                _ => "?".to_string(),
            }
        };
        Self {
            input: String::new(),
            input_cursor: 0,
            input_scroll: 0,
            cmd_line: String::new(),
            messages: Vec::new(),
            visible_indices: Vec::new(),
            raw_messages: Vec::new(),
            plan: PlanPanel::new(),
            status: Status::Idle,
            agent_rx,
            user_cmd_tx,
            task_history: Vec::new(),
            theme: Theme::by_name(ThemeName::Nord),
            log_scroll: LogScroll::new(),
            show_history: false,
            show_help: false,
            focused_panel: FocusedPanel::Log,
            mouse: MouseState::new(),
            input_mode: InputMode::Insert,
            palette_selected: 0,
            search: SearchState::new(),
            command_history: Vec::new(),
            input_history: InputHistory::new(Self::load_history(&work_dir)),
            work_dir,
            should_quit: false,
            dirty: true,
            clipboard_buffer: String::new(),
            status_bar: StatusBarState::new(git_branch),
            task_start_time: None,
            task_done_time: None,
            process_start_time: chrono::Local::now(),
            workspace_dir,
            select: SelectPopup::new(),
            diff_blocks: Vec::new(),
            diff_popup: None,
            stream: StreamState::new(),
            thinking: ThinkingState::new(),
            balance_info: None,
            party_mode: false,
            konami_progress: 0,
            language: Language::English,
            flash_msg: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// 处理来自 Agent 的状态更新，同步 UI 状态、消息列表和滚动条。
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
                        // 闭合 ``` → 移除流式指示符，用语法高亮版本替换增量渲染的行
                        let last_idx = self.messages.len().saturating_sub(1);
                        if let Some(last_raw) = self.raw_messages.get_mut(last_idx) {
                            if last_raw.ends_with(STREAMING_INDICATOR) {
                                let trimmed_raw = last_raw
                                    .trim_end_matches(STREAMING_INDICATOR)
                                    .to_string();
                                *last_raw = trimmed_raw.clone();
                                self.messages[last_idx] = Line::from(Span::styled(
                                    trimmed_raw,
                                    Style::default().fg(CODE_FG).bg(CODE_BG),
                                ));
                            }
                        }

                        let lang = std::mem::take(&mut self.stream.code_block_lang);
                        let lines = std::mem::take(&mut self.stream.code_block_buffer);
                        let code_text = format!("```{}\n{}\n```", lang, lines.join("\n"));

                        if let Some(start_idx) = self.stream.code_block_start_idx.take() {
                            let line_count = self.stream.code_block_line_count;
                            let end_idx = start_idx + line_count;
                            if !code_text.trim().is_empty() {
                                let (styled, raw) = render_markdown_tui(&code_text);
                                let _: Vec<_> =
                                    self.messages.splice(start_idx..end_idx, styled).collect();
                                let _: Vec<_> =
                                    self.raw_messages.splice(start_idx..end_idx, raw).collect();
                            } else {
                                self.messages.drain(start_idx..end_idx);
                                self.raw_messages.drain(start_idx..end_idx);
                            }
                        } else if !code_text.trim().is_empty() {
                            let (styled, raw) = render_markdown_tui(&code_text);
                            completed.extend(styled.into_iter().zip(raw));
                        }
                        self.stream.code_block = false;
                        self.stream.code_block_line_count = 0;
                    } else if self.stream.code_block {
                        // 代码块内部：增量渲染，逐行显示
                        self.stream.code_block_buffer.push(line.clone());

                        let prev_idx = self.messages.len().saturating_sub(1);
                        if self.stream.code_block_line_count > 1 {
                            if let Some(prev_raw) = self.raw_messages.get_mut(prev_idx) {
                                if prev_raw.ends_with(STREAMING_INDICATOR) {
                                    let trimmed_raw = prev_raw
                                        .trim_end_matches(STREAMING_INDICATOR)
                                        .to_string();
                                    *prev_raw = trimmed_raw.clone();
                                    self.messages[prev_idx] = Line::from(Span::styled(
                                        trimmed_raw,
                                        Style::default().fg(CODE_FG).bg(CODE_BG),
                                    ));
                                }
                            }
                        }

                        let display_text = format!("{}{}", line, STREAMING_INDICATOR);
                        self.messages.push(Line::from(Span::styled(
                            display_text,
                            Style::default().fg(CODE_FG).bg(CODE_BG),
                        )));
                        self.raw_messages.push(line);
                        self.stream.code_block_line_count += 1;
                    } else if is_code_fence {
                        // 开启新的代码块：先 flush 之前累积的内容
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

                        // flush completed 行，确保 start_idx 计算准确
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

                        let header_text = format!("```{}", lang);
                        self.messages.push(Line::from(Span::styled(
                            header_text.clone(),
                            Style::default().fg(Color::Gray).bg(CODE_BG),
                        )));
                        self.raw_messages.push(header_text);
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

    /// 在 Log 区域输出启动 Logo（ASCII 艺术 "T" + 标语），仅在 TUI 启动时调用一次。
    pub(crate) fn add_startup_logo(&mut self) {
        let logo = [
            "  ████████╗ ",
            "  ╚══██╔══╝ ",
            "     ██║    ",
            "     ██║    ",
            "     ██║    ",
            "     ╚═╝    ",
        ];

        self.add_new_line();
        for line in &logo {
            self.messages.push(Line::from(Span::styled(
                (*line).to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )));
            self.raw_messages.push((*line).to_string());
        }

        let title = "  Tact Agent";
        self.messages.push(Line::from(Span::styled(
            title.to_string(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
        self.raw_messages.push(title.to_string());

        let tagline = "  thoughtful communication";
        self.messages.push(Line::from(Span::styled(
            tagline.to_string(),
            Style::default()
                .fg(Color::Rgb(128, 128, 128))
                .add_modifier(Modifier::ITALIC),
        )));
        self.raw_messages.push(tagline.to_string());
        self.add_new_line();
    }

    /// 保存当前输入状态到 undo 栈，并清空 redo 栈。最多保留 100 条快照。
    pub(crate) fn save_undo(&mut self) {
        self.redo_stack.clear();
        self.undo_stack.push((self.input.clone(), self.input_cursor));
        if self.undo_stack.len() > 100 {
            self.undo_stack.remove(0);
        }
    }

    /// 添加一条系统消息，根据前缀自动着色，并更新滚动位置。
    /// 非系统标记消息会被解析为 Markdown。
    pub(crate) fn add_system_message(&mut self, content: String) {
        let trimmed = content.trim_start();
        let is_system = trimmed.starts_with('✓')
            || trimmed.starts_with('✗')
            || trimmed.starts_with('⚠')
            || trimmed.starts_with('📝')
            || trimmed.starts_with('❌')
            || trimmed.starts_with('✅')
            || trimmed.starts_with('▶')
            || trimmed.starts_with('🤖')
            || trimmed.starts_with("  ");

        if is_system {
            let color = if content.starts_with('✓') {
                self.theme.success
            } else if content.starts_with('✗') {
                self.theme.error
            } else if content.starts_with('⚠') {
                self.theme.warning
            } else {
                self.theme.accent
            };
            for line in content.split('\n') {
                self.messages.push(Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(color),
                )));
                self.raw_messages.push(line.to_string());
            }
        } else {
            let (lines, raw_lines) = render_markdown_tui(&content);
            self.messages.extend(lines);
            self.raw_messages.extend(raw_lines);
        }

        if self.input_mode == InputMode::Insert || self.input_mode == InputMode::Normal {
            // u16::MAX 会被 render_log_panel 按视觉行数正确裁剪
            self.log_scroll.offset = u16::MAX;
        }
        if !self.search.term.is_empty() {
            self.update_search_matches();
        }
    }

    /// 添加一条用户输入消息，同时将其记录到任务历史中。
    pub(crate) fn add_user_message(&mut self, content: String) {
        // 先插入一个空行作为分隔
        self.add_new_line();
        let mut is_first = true;
        let msgs = self.msgs();
        for line in content.split('\n') {
            let text = if is_first {
                msgs.user_msg_prefix.replace("{}", line)
            } else {
                msgs.user_msg_cont.replace("{}", line)
            };
            self.messages.push(Line::from(Span::styled(
                text.clone(),
                Style::default().fg(self.theme.success),
            )));
            self.raw_messages.push(text);
            is_first = false;
        }
        let timestamp = Local::now().format("%H:%M:%S").to_string();
        self.task_history.push(HistoryEntry {
            task: content,
            timestamp,
            summary: String::new(),
        });
        if self.task_history.len() > 20 {
            self.task_history.remove(0);
        }
    }

    /// 切换到下一个内置主题。
    /// 从 `.tact/history.txt` 加载输入历史。
    fn load_history(work_dir: &Path) -> Vec<String> {
        let path = work_dir.join(".tact").join("history.txt");
        std::fs::read_to_string(&path)
            .map(|s| s.lines().map(|l| l.to_string()).collect())
            .unwrap_or_default()
    }

    /// 将当前内存中的输入历史写入 `.tact/history.txt`。
    pub(crate) fn save_history(&self) {
        let dir = self.work_dir.join(".tact");
        if !dir.exists() {
            let _ = std::fs::create_dir_all(&dir);
        }
        let path = dir.join("history.txt");
        let data = self.input_history.entries.join("\n");
        let _ = std::fs::write(&path, data);
    }

    pub(crate) fn toggle_theme(&mut self) {
        let next_name = self.theme.name.next();
        let msgs = self.msgs();
        let label = match next_name {
            crate::theme::ThemeName::Dark => msgs.theme_dark,
            crate::theme::ThemeName::Light => msgs.theme_light,
            crate::theme::ThemeName::SolarizedDark => msgs.theme_solarized_dark,
            crate::theme::ThemeName::SolarizedLight => msgs.theme_solarized_light,
            crate::theme::ThemeName::GruvboxDark => msgs.theme_gruvbox_dark,
            crate::theme::ThemeName::Nord => msgs.theme_nord,
            crate::theme::ThemeName::Retro => msgs.theme_retro,
            crate::theme::ThemeName::Kawaii => msgs.theme_kawaii,
            crate::theme::ThemeName::Japanese => msgs.theme_japanese,
        };
        self.add_system_message(msgs.theme_changed_tmpl.replace("{}", label));
        self.theme = Theme::by_name(next_name);
    }

    /// 返回当前语言对应的 UI 字符串集合。
    pub(crate) fn msgs(&self) -> Messages {
        Messages::by_language(self.language)
    }

    /// 将命令面板命令名翻译为当前语言的描述文本。
    pub(crate) fn localize_cmd_desc(&self, cmd: &str) -> String {
        let msgs = self.msgs();
        match cmd {
            "theme" => msgs.cmd_theme.to_string(),
            "save" => msgs.cmd_save.to_string(),
            "cancel" => msgs.cmd_cancel.to_string(),
            "quit" => msgs.cmd_quit.to_string(),
            "help" => msgs.cmd_help.to_string(),
            "history" => msgs.cmd_history.to_string(),
            "search" => msgs.cmd_search.to_string(),
            "balance" => msgs.cmd_balance.to_string(),
            "lang" => msgs.cmd_lang.to_string(),
            _ => cmd.to_string(),
        }
    }

    /// 循环切换界面语言。
    pub(crate) fn toggle_language(&mut self) {
        let next = self.language.next();
        let label = next.label();
        let old_msgs = self.msgs();
        self.language = next;
        self.add_system_message(old_msgs.lang_changed_tmpl.replace("{}", label));
    }

    /// 切换派对模式 —— Konami Code 彩蛋 🎉
    pub(crate) fn toggle_party_mode(&mut self) {
        self.party_mode = !self.party_mode;
        let msgs = self.msgs();
        if self.party_mode {
            // 创建彩虹色 spans
            let colors = [
                Color::Rgb(255, 105, 180), // Hot Pink
                Color::Rgb(255, 165, 0),   // Orange
                Color::Rgb(255, 215, 0),   // Gold
                Color::Rgb(50, 205, 50),   // Lime Green
                Color::Rgb(0, 191, 255),   // Deep Sky Blue
                Color::Rgb(138, 43, 226),  // Blue Violet
                Color::Rgb(255, 0, 255),   // Magenta
            ];

            let cat_art = [
                "  ╱|、",
                " (˚ˎ 。7  ",
                "  |、˜\\\\",
                " じしˍ,)ノ",
                "",
                msgs.party_msg_1,
                msgs.party_msg_2,
                msgs.party_msg_3,
                "",
                msgs.party_hint,
            ];

            // 在消息列表中展示猫猫艺术
            self.add_new_line();
            for (line_num, &line) in cat_art.iter().enumerate() {
                let color = colors[line_num % colors.len()];
                self.messages.push(Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )));
                self.raw_messages.push(line.to_string());
            }
            self.add_new_line();
        } else {
            // 告别消息
            self.add_new_line();
            self.messages.push(Line::from(Span::styled(
                msgs.party_exit,
                Style::default()
                    .fg(Color::Rgb(180, 180, 180))
                    .add_modifier(Modifier::ITALIC),
            )));
            self.raw_messages.push(msgs.party_exit.to_string());
            self.add_new_line();
        }
    }

    /// 判断指定物理索引的消息行是否可见（未被折叠的 thinking 内容隐藏）。
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
    fn close_active_thinking_block(&mut self) {
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
    fn flush_and_close_thinking(&mut self) {
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
    fn flush_stream_pending(&mut self) {
        // flush 累积的表格
        if !self.stream.table_buffer.is_empty() {
            let (lines, raw_lines) = format_table(&self.stream.table_buffer, &self.theme);
            self.messages.extend(lines);
            self.raw_messages.extend(raw_lines);
            self.stream.table_buffer.clear();
        }
        // flush 未闭合的代码块（流中断时可能残留）
        if self.stream.code_block {
            // 移除最后一行的流式指示符
            let last_idx = self.messages.len().saturating_sub(1);
            if let Some(last_raw) = self.raw_messages.get_mut(last_idx) {
                if last_raw.ends_with(STREAMING_INDICATOR) {
                    let trimmed_raw = last_raw
                        .trim_end_matches(STREAMING_INDICATOR)
                        .to_string();
                    *last_raw = trimmed_raw.clone();
                    self.messages[last_idx] = Line::from(Span::styled(
                        trimmed_raw,
                        Style::default().fg(CODE_FG).bg(CODE_BG),
                    ));
                }
            }

            let lang = std::mem::take(&mut self.stream.code_block_lang);
            let code_lines = std::mem::take(&mut self.stream.code_block_buffer);
            let code_text = format!("```{}\n{}\n```", lang, code_lines.join("\n"));

            if let Some(start_idx) = self.stream.code_block_start_idx.take() {
                let line_count = self.stream.code_block_line_count;
                let end_idx = start_idx + line_count;
                if !code_text.trim().is_empty() {
                    let (styled, raw) = render_markdown_tui(&code_text);
                    let _: Vec<_> =
                        self.messages.splice(start_idx..end_idx, styled).collect();
                    let _: Vec<_> =
                        self.raw_messages.splice(start_idx..end_idx, raw).collect();
                } else {
                    self.messages.drain(start_idx..end_idx);
                    self.raw_messages.drain(start_idx..end_idx);
                }
            } else if !code_text.trim().is_empty() {
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

    pub(crate) fn update_search_matches(&mut self) {
        self.search.matches.clear();
        let mut logical_idx = 0;
        for (idx, msg) in self.raw_messages.iter().enumerate() {
            if !self.is_message_visible(idx) {
                continue;
            }
            if msg
                .to_lowercase()
                .contains(&self.search.term.to_lowercase())
            {
                self.search.matches.push(logical_idx);
            }
            logical_idx += 1;
        }
        // 未完成的流式行也可能匹配
        if !self.stream.buffer.is_empty()
            && self
                .stream
                .buffer
                .to_lowercase()
                .contains(&self.search.term.to_lowercase())
        {
            self.search.matches.push(logical_idx);
        }
        if !self.search.matches.is_empty() {
            self.search.current_match = 0;
            if let Some(&match_idx) = self.search.matches.first() {
                self.log_scroll.offset =
                    (match_idx as u16).saturating_sub(self.log_scroll.height / 2);
            }
        }
    }

    /// 跳转到下一个搜索匹配项并调整滚动位置。
    pub(crate) fn jump_to_next_match(&mut self) {
        if self.search.matches.is_empty() {
            return;
        }
        self.search.current_match = (self.search.current_match + 1) % self.search.matches.len();
        let target_line = self.search.matches[self.search.current_match];
        self.log_scroll.offset = (target_line as u16).saturating_sub(self.log_scroll.height / 2);
    }

    /// 跳转到上一个搜索匹配项并调整滚动位置。
    pub(crate) fn jump_to_prev_match(&mut self) {
        if self.search.matches.is_empty() {
            return;
        }
        self.search.current_match = if self.search.current_match == 0 {
            self.search.matches.len() - 1
        } else {
            self.search.current_match - 1
        };
        let target_line = self.search.matches[self.search.current_match];
        self.log_scroll.offset = (target_line as u16).saturating_sub(self.log_scroll.height / 2);
    }

    /// 重新提交历史任务，清空当前计划并发送给 Agent。
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
