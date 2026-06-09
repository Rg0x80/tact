/// 输入历史状态，支持上下键回溯之前提交过的输入。
pub(crate) struct InputHistory {
    pub(crate) entries: Vec<String>,
    /// 导航的当前位置；None 表示未进入导航模式。
    pub(crate) index: Option<usize>,
    /// 进入导航前用户正在编辑的输入（用于 ESC 返回原位）。
    pub(crate) saved: String,
}

impl InputHistory {
    pub(crate) fn new(entries: Vec<String>) -> Self {
        Self {
            entries,
            index: None,
            saved: String::new(),
        }
    }
}
