use crate::state::App;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
};

/// 主内容区域布局，根据当前显示状态切换历史、帮助或 Plan+Log 双栏布局。
pub(crate) fn render_main_area(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.show_history {
        super::popup::render_history_panel(frame, area, app);
        return;
    }
    if app.show_help {
        super::popup::render_help_panel(frame, area, app);
        return;
    }
    if app.plan.visible {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(20), Constraint::Percentage(80)])
            .split(area);
        app.mouse.plan_area = chunks[0];
        app.mouse.log_area = chunks[1];
        super::plan::render_plan_panel(frame, chunks[0], app);
        super::log::render_log_panel(frame, chunks[1], app);
    } else {
        app.mouse.plan_area = Rect::new(0, 0, 0, 0);
        app.mouse.log_area = area;
        super::log::render_log_panel(frame, area, app);
    }

    if app.thinking.popup.is_some() {
        super::popup::render_thinking_popup(frame, area, app);
    }
    if app.diff_popup.is_some() {
        super::popup::render_diff_popup(frame, area, app);
    }
}
