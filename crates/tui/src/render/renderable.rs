use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// 可渲染单元，知道自己的视觉高度和如何绘制。
pub(crate) trait Renderable {
    /// 在指定区域内绘制。
    fn render(&self, area: Rect, buf: &mut Buffer);
    /// 在给定宽度下的视觉行数（折行后高度）。
    fn height(&self, width: u16) -> u16;
}
