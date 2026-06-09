/// 选择弹窗状态，独立管理 prompt、选项、选中索引和响应通道。
pub(crate) struct SelectPopup {
    /// 弹窗提示文本。
    pub(crate) prompt: String,
    /// 选项列表。
    pub(crate) options: Vec<String>,
    /// 当前选中的选项索引。
    pub(crate) selected: usize,
    /// 响应通道，用于将选中的选项索引发送回调用方。
    pub(crate) respond: Option<tokio::sync::oneshot::Sender<Option<usize>>>,
}

impl SelectPopup {
    pub(crate) fn new() -> Self {
        Self {
            prompt: String::new(),
            options: Vec::new(),
            selected: 0,
            respond: None,
        }
    }

    /// 设置弹窗内容并激活。
    pub(crate) fn set(
        &mut self,
        prompt: String,
        options: Vec<String>,
        respond: tokio::sync::oneshot::Sender<Option<usize>>,
    ) {
        self.prompt = prompt;
        self.options = options;
        self.selected = 0;
        self.respond = Some(respond);
    }

    /// 确认当前选择，发送选中索引并清空 respond。
    pub(crate) fn confirm(&mut self) -> Option<usize> {
        let respond = self.respond.take();
        let idx = self.selected.min(self.options.len().saturating_sub(1));
        if let Some(tx) = respond {
            let _ = tx.send(Some(idx));
        }
        Some(idx)
    }

    /// 取消选择，发送 None 并清空 respond。
    pub(crate) fn cancel(&mut self) {
        if let Some(tx) = self.respond.take() {
            let _ = tx.send(None);
        }
    }

    /// 选中项下移。
    pub(crate) fn move_down(&mut self) {
        if self.selected + 1 < self.options.len() {
            self.selected += 1;
        }
    }

    /// 选中项上移。
    pub(crate) fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }
}
