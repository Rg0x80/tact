use crate::i18n::Language;
use crate::theme::Theme;
use chrono;
use ratatui::text::Line;
use std::path::PathBuf;
use tact_core::{AgentUpdate, UserCommand};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub(crate) use tact_core::PlanStep;

mod app;
mod input_history;
mod log_scroll;
mod mouse_state;
mod plan_panel;
pub(crate) mod render_md;
mod search_state;
mod select_popup;
mod status_bar_state;
mod stream_state;
mod thinking_state;

pub(crate) use input_history::InputHistory;
pub(crate) use log_scroll::LogScroll;
pub(crate) use mouse_state::MouseState;
pub(crate) use plan_panel::PlanPanel;
pub(crate) use search_state::SearchState;
pub(crate) use select_popup::SelectPopup;
pub(crate) use status_bar_state::StatusBarState;
pub(crate) use stream_state::StreamState;
pub(crate) use thinking_state::{ThinkingBlock, ThinkingPopup, ThinkingState};

// ========== 基础类型 ==========

/// 当前键盘输入模式，决定按键行为的解释方式。
#[derive(PartialEq)]
pub(crate) enum InputMode {
    Normal,
    Insert,
    Search,
    Palette,
    Select,
}

/// Commands shown in the command palette (triggered by `:`).
pub(crate) const PALETTE_COMMANDS: &[(&str, &str)] = &[
    ("theme", "Toggle color theme"),
    ("save", "Save log to file"),
    ("cancel", "Cancel current task"),
    ("quit", "Quit application"),
    ("help", "Show help panel"),
    ("history", "Show task history"),
    ("search", "Search log messages"),
    ("balance", "Query account balance (DeepSeek)"),
    ("lang", "Toggle language (EN/中文)"),
];

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum FocusedPanel {
    Plan,
    Log,
}

#[derive(Clone)]
pub struct HistoryEntry {
    pub task: String,
    pub timestamp: String,
    pub summary: String,
}

// ========== Diff 类型 ==========

/// 文件写入 diff 块信息。
#[derive(Debug, Clone)]
pub(crate) struct DiffBlock {
    /// diff 块的首行索引（在 messages 中）。
    pub start_idx: usize,
    /// diff 块的末行索引（在 messages 中，不包含）。
    pub end_idx: usize,
    pub file_path: String,
    pub content: String,
}

/// 文件写入内容的弹窗预览状态。
#[derive(Debug, Clone)]
pub(crate) struct DiffPopup {
    pub block_idx: usize,
    pub file_path: String,
    pub content: String,
    pub scroll: u16,
}

// ========== 执行状态 ==========

/// Agent 当前的执行状态，用于驱动状态栏和 UI 反馈。
pub(crate) enum Status {
    Idle,
    Planning,
    Executing {
        current_step: usize,
        total: usize,
    },
    WaitingForUser {
        prompt: String,
        step_index: usize,
        approval_tx: tokio::sync::oneshot::Sender<bool>,
    },
    Done,
}

// ========== 主状态 ==========

/// TUI 应用主状态，持有所有 UI 状态、滚动位置、通信通道及当前模式。
pub struct App {
    // 输入
    pub(crate) input: String,
    pub(crate) input_cursor: usize,
    pub(crate) input_scroll: u16,
    pub(crate) cmd_line: String,
    pub(crate) messages: Vec<Line<'static>>,
    /// 可见索引缓存：logical line → physical msg index。由 render_log_panel 每帧重建。
    pub(crate) visible_indices: Vec<usize>,
    pub(crate) raw_messages: Vec<String>,
    pub(crate) plan: PlanPanel,
    pub(crate) status: Status,
    pub(crate) agent_rx: UnboundedReceiver<AgentUpdate>,
    pub(crate) user_cmd_tx: UnboundedSender<UserCommand>,
    pub(crate) task_history: Vec<HistoryEntry>,
    pub(crate) theme: Theme,
    // 滚动
    pub(crate) log_scroll: LogScroll,
    // 面板
    pub(crate) show_history: bool,
    pub(crate) show_help: bool,
    pub(crate) focused_panel: FocusedPanel,
    // 鼠标交互
    pub(crate) mouse: MouseState,
    // 模式
    pub(crate) input_mode: InputMode,
    // 命令面板
    pub(crate) palette_selected: usize,
    // 搜索
    pub(crate) search: SearchState,
    // 命令历史（简略）
    pub(crate) command_history: Vec<String>,
    /// 用户输入历史。
    pub(crate) input_history: InputHistory,
    /// 项目根目录，用于读写 .tact/history.txt。
    pub(crate) work_dir: PathBuf,
    pub(crate) should_quit: bool,
    /// 脏标记：当有输入事件、agent 更新或尺寸变化时置 true，空闲时跳过无意义的重绘。
    pub(crate) dirty: bool,
    /// 内部剪贴板缓冲区（当系统剪贴板不可用时使用）
    pub(crate) clipboard_buffer: String,
    // 底部状态栏
    pub(crate) status_bar: StatusBarState,
    /// 当前任务开始时间（用于底部状态栏计时）
    pub(crate) task_start_time: Option<chrono::DateTime<chrono::Local>>,
    /// 任务完成时间（用于顶部状态栏 Done 高亮计时，2s 后自动恢复 Idle 显示）
    pub(crate) task_done_time: Option<chrono::DateTime<chrono::Local>>,
    /// 进程启动时间（用于底部状态栏显示 TUI 总运行时间）
    pub(crate) process_start_time: chrono::DateTime<chrono::Local>,
    /// 当前工作目录
    pub(crate) workspace_dir: String,
    /// 文件写入 diff 块列表。
    pub(crate) diff_blocks: Vec<DiffBlock>,
    /// 文件写入内容的弹窗预览。
    pub(crate) diff_popup: Option<DiffPopup>,
    // 选择弹窗
    pub(crate) select: SelectPopup,
    // 流式输出状态
    pub(crate) stream: StreamState,
    // Thinking 状态
    pub(crate) thinking: ThinkingState,
    /// DeepSeek 账户余额信息（页面加载时查询一次并缓存）
    pub(crate) balance_info: Option<tact_core::BalanceInfo>,
    /// 派对模式：Konami Code 触发的彩蛋
    pub(crate) party_mode: bool,
    /// Konami Code 输入进度 (0=未开始, 1-10=进度, 10=触发)
    pub(crate) konami_progress: u8,
    /// 当前界面语言。
    pub(crate) language: Language,
    /// 短暂状态栏通知（显示 3s 后自动消失）
    pub(crate) flash_msg: Option<(String, std::time::Instant)>,
    /// 输入框 undo 栈（最多 100 条，每次变更前保存快照）
    pub(crate) undo_stack: Vec<(String, usize)>,
    /// 输入框 redo 栈
    pub(crate) redo_stack: Vec<(String, usize)>,
}
