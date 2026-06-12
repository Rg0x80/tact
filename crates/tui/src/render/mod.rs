//! Render module — split by panel type.

mod bar;
pub(crate) mod cells;
mod input;
mod layout;
mod log;
mod log_column;
mod plan;
mod popup;
pub(crate) mod renderable;
mod util;
pub(crate) mod welcome;

pub(super) use bar::{render_bottom_bar, render_status_bar};
pub(super) use input::render_input_box;
pub(super) use layout::render_main_area;
pub(super) use popup::{render_command_palette, render_select_popup};
