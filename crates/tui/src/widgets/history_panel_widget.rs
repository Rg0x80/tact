use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, Widget};
use crate::state::HistoryEntry;

/// 任务历史面板 Widget，按时间倒序展示历史记录，支持回车重试。
pub struct HistoryPopupWidget<'a> {
    history: &'a [HistoryEntry],
    /// 条目前景色。
    accent_color: Color,
    /// 边框色。
    border_color: Color,
    /// 面板标题（i18n）。
    title: &'static str,
}

impl<'a> HistoryPopupWidget<'a> {
    pub fn new(
        history: &'a [HistoryEntry],
        accent_color: Color,
        border_color: Color,
        title: &'static str,
    ) -> Self {
        HistoryPopupWidget {
            history,
            accent_color,
            border_color,
            title,
        }
    }
}

impl Widget for HistoryPopupWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized
    {
        let items: Vec<ListItem> = self
            .history
            .iter()
            .rev()
            .map(|entry| {
                let mut text = format!("[{}] {}", entry.timestamp, entry.task);
                if !entry.summary.is_empty() {
                    text.push_str(&format!(" -> {}", entry.summary));
                }
                ListItem::new(text).style(Style::default().fg(self.accent_color))
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(self.border_color))
                .title(self.title),
        );
        list.render(area, buf);
    }
}
