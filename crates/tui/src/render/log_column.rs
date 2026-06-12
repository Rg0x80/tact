use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;
use super::renderable::Renderable;

/// 日志列布局渲染器，负责将 Renderable 单元按视觉 offset 排列并绘制。
pub(crate) struct LogColumnRenderer<'a> {
    /// (视觉起始行, 渲染单元) 的列表，按视觉行递增排列。
    cells: Vec<(usize, Box<dyn Renderable + 'a>)>,
    /// 视口顶部视觉行号。
    viewport_top: usize,
    /// 视口可见行数。
    viewport_height: usize,
}

impl<'a> LogColumnRenderer<'a> {
    pub(crate) fn new() -> Self {
        LogColumnRenderer {
            cells: Vec::new(),
            viewport_top: 0,
            viewport_height: 0,
        }
    }

    pub(crate) fn with_viewport(mut self, top: usize, height: usize) -> Self {
        self.viewport_top = top;
        self.viewport_height = height;
        self
    }

    pub(crate) fn push(&mut self, vis_start: usize, cell: impl Renderable + 'a) {
        self.cells.push((vis_start, Box::new(cell)));
    }
}

impl Widget for LogColumnRenderer<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let viewport_bottom = self.viewport_top + self.viewport_height;
        for (vis_start, cell) in &self.cells {
            let cell_height = cell.height(area.width) as usize;
            let vis_end = vis_start + cell_height;
            if vis_end <= self.viewport_top || *vis_start >= viewport_bottom {
                continue;
            }

            // 计算可见部分
            let visible_start = (*vis_start).max(self.viewport_top);
            let visible_end = vis_end.min(viewport_bottom);
            let skip_lines = visible_start - vis_start;
            let visible_lines = visible_end - visible_start;

            let y = area.y + (visible_start - self.viewport_top) as u16;
            let cell_area = Rect::new(area.x, y, area.width, visible_lines as u16);

            // 只渲染视口内的行：从 skip_lines 开始，最多 visible_lines 行
            cell.render_partial(cell_area, buf, skip_lines);
        }
    }
}
