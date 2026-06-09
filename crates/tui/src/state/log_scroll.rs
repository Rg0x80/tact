use ratatui::text::Line;
use ratatui::widgets::ScrollbarState;

/// 日志面板滚动状态，管理滚动偏移、滚动条、面板高度和视觉行映射。
pub(crate) struct LogScroll {
    /// 当前滚动偏移量。
    pub(crate) offset: u16,
    /// 滚动条状态。
    pub(crate) state: ScrollbarState,
    /// 面板高度。
    pub(crate) height: u16,
    /// 视觉行起始索引列表。
    pub(crate) visual_start: Vec<usize>,
    /// 缓存的视觉行（wrap_line 结果，不含搜索/选择样式）。
    pub(crate) visual_cache: Vec<Line<'static>>,
    /// 缓存的逻辑→视觉映射（visual_cache 的起始索引）。
    pub(crate) visual_start_cache: Vec<usize>,
    /// 缓存的视觉行宽度。
    pub(crate) visual_cache_width: u16,
    /// 上次构建缓存时的 messages.len()，变动则失效。
    pub(crate) visual_cache_ver: usize,
    /// visible_indices 上次构建时的 messages.len()。
    pub(crate) visible_indices_ver: usize,
    /// physical → logical 反向映射缓存（用 Option 处理不可见行）。
    pub(crate) phys_to_logical_cache: Vec<Option<usize>>,
}

impl LogScroll {
    pub(crate) fn new() -> Self {
        Self {
            offset: 0,
            state: ScrollbarState::new(0),
            height: 10,
            visual_start: Vec::new(),
            visual_cache: Vec::new(),
            visual_start_cache: Vec::new(),
            visual_cache_width: 0,
            visual_cache_ver: 0,
            visible_indices_ver: 0,
            phys_to_logical_cache: Vec::new(),
        }
    }
}
