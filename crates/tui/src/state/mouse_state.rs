use ratatui::layout::Rect;

/// 鼠标交互状态，管理面板区域、选择范围和拖拽标志。
#[derive(Default)]
pub(crate) struct MouseState {
    pub(crate) plan_area: Rect,
    pub(crate) log_area: Rect,
    pub(crate) plan_selection: Option<(usize, usize)>,
    pub(crate) dragging_plan: bool,
    pub(crate) log_selection: Option<(usize, usize)>,
    pub(crate) dragging_log: bool,
    /// thinking 弹窗区域（用于判断点击是否在弹窗内部）。
    pub(crate) thinking_popup_area: Rect,
    /// diff 弹窗区域（用于判断点击是否在弹窗内部）。
    pub(crate) diff_popup_area: Rect,
    /// 代码块弹窗区域（用于判断点击是否在弹窗内部）。
    pub(crate) code_popup_area: Rect,
    /// 双击/三击检测：上次左键点击的时间和位置。
    pub(crate) last_click_time: Option<std::time::Instant>,
    pub(crate) last_click_pos: Option<(u16, u16)>,
    /// 连续点击次数（1=单击，2=双击，3=三击）。
    pub(crate) click_count: u8,
    /// 双击选词：记录 (词起始字节, 词结束字节)，配合 log_selection 的 line 使用。
    pub(crate) log_word_selection: Option<(usize, usize)>,
    /// 上次点击命中的 thinking 块索引（用于双击打开弹窗）。
    pub(crate) last_click_card: Option<usize>,
    /// 上次点击命中的 diff 块索引（用于双击打开弹窗）。
    pub(crate) last_click_diff: Option<usize>,
    /// 上次点击命中的代码块索引（用于双击打开弹窗）。
    pub(crate) last_click_code: Option<usize>,
}

impl MouseState {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}
