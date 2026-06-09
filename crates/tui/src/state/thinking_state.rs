/// Thinking 状态，管理 reasoning 内容缓冲区、标题标记、活跃/已完成块和弹窗。
pub(crate) struct ThinkingState {
    /// 推理内容缓冲区。
    pub(crate) buffer: String,
    /// 是否已添加标题。
    pub(crate) title_added: bool,
    /// 活跃块的起始位置。
    pub(crate) active_start: Option<usize>,
    /// 活跃块的结束位置。
    pub(crate) active_end: Option<usize>,
    /// 推理块列表。
    pub(crate) blocks: Vec<ThinkingBlock>,
    /// 弹窗状态。
    pub(crate) popup: Option<ThinkingPopup>,
}

/// 一个已完成的 Thinking 块在 messages 中的范围及滚动状态。
/// 完成后默认只展示最后 3 行，scroll_offset 控制可见窗口的起始行。
#[derive(Debug, Clone)]
pub(crate) struct ThinkingBlock {
    pub title_idx: usize,
    pub end_idx: usize,
    /// 当前可见窗口的起始行偏移（相对于 title_idx+1），默认自动滚动到底部。
    pub scroll_offset: usize,
    /// 缓存的纯文本行（已去除 "│ " 前缀），用于卡片预览与复制。
    pub(crate) cached_preview: Vec<String>,
    /// 缓存的 Markdown 渲染行，用于弹窗展示，避免每帧重复渲染。
    pub(crate) cached_markdown: Vec<ratatui::text::Line<'static>>,
}

/// Thinking 弹窗状态。
#[derive(Debug, Clone)]
pub(crate) struct ThinkingPopup {
    pub block_idx: usize,
    pub title: String,
    /// 弹窗内部滚动偏移（行号，相对于 thinking 内容首行）。
    pub scroll: u16,
}

impl ThinkingState {
    pub(crate) fn new() -> Self {
        Self {
            buffer: String::new(),
            title_added: false,
            active_start: None,
            active_end: None,
            blocks: Vec::new(),
            popup: None,
        }
    }
}
