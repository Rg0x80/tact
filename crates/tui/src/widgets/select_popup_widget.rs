use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Widget};
use crate::state::SelectPopup;

/// 选择弹窗 Widget，居中显示 prompt 和选项列表，支持键盘/鼠标选中。
pub struct SelectPopupWidget<'a> {
    state: &'a SelectPopup,
    /// 选中项高亮背景色。
    highlight_color: Color,
    /// 普通选项前景色。
    fg_color: Color,
    /// 弹窗背景色。
    bg_color: Color,
    /// 无选项时的提示文本。
    empty_text: &'static str,
    /// 选中项前缀箭头。
    arrow: &'static str,
}

impl<'a> SelectPopupWidget<'a> {
    pub fn new(
        state: &'a SelectPopup,
        highlight_color: Color,
        fg_color: Color,
        bg_color: Color,
        empty_text: &'static str,
        arrow: &'static str,
    ) -> Self {
        SelectPopupWidget {
            state,
            highlight_color,
            fg_color,
            bg_color,
            empty_text,
            arrow,
        }
    }
}

impl Widget for SelectPopupWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized
    {
        let count = self.state.options.len().max(1) as u16;
        let popup_width = 50u16.min(area.width.saturating_sub(4));
        let popup_height = (count + 4).min(area.height.saturating_sub(4));
        let popup_x = (area.width.saturating_sub(popup_width)) / 2;
        let popup_y = (area.height.saturating_sub(popup_height)) / 2;
        let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

        // 清除弹窗区域原有内容
        Clear.render(popup_area, buf);

        // 带边框的弹窗外框
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} ", self.state.prompt))
            .style(Style::default().bg(self.bg_color));
        block.render(popup_area, buf);

        // 弹窗内部区域
        let inner = Rect::new(
            popup_area.x + 1,
            popup_area.y + 1,
            popup_area.width.saturating_sub(2),
            popup_area.height.saturating_sub(2),
        );

        // 构建选项列表
        let items: Vec<ListItem> = if self.state.options.is_empty() {
            vec![ListItem::new(Span::styled(
                self.empty_text,
                Style::default().fg(Color::Gray),
            ))]
        } else {
            let selected = self
                .state
                .selected
                .min(self.state.options.len().saturating_sub(1));
            self.state
                .options
                .iter()
                .enumerate()
                .map(|(i, opt)| {
                    let is_selected = i == selected;
                    let style = if is_selected {
                        Style::default()
                            .bg(self.highlight_color)
                            .fg(Color::White)
                    } else {
                        Style::default().fg(self.fg_color)
                    };
                    let prefix = if is_selected { self.arrow } else { "  " };
                    ListItem::new(Span::styled(
                        format!("{}{}", prefix, opt),
                        style,
                    ))
                })
                .collect()
        };

        let list = List::new(items).block(Block::default());
        list.render(inner, buf);
    }
}
