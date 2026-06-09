/// 搜索状态，管理搜索词、匹配行索引和当前高亮项。
pub(crate) struct SearchState {
    pub(crate) term: String,
    pub(crate) matches: Vec<usize>,
    pub(crate) current_match: usize,
}

impl SearchState {
    pub(crate) fn new() -> Self {
        Self {
            term: String::new(),
            matches: Vec::new(),
            current_match: 0,
        }
    }
}
