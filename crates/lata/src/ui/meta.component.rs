//! A small icon + text pair for metadata rows (branch, last played...), instead of
//! separator characters between values.

use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::*;

#[derive(IntoElement)]
pub struct Meta {
    icon: Icon,
    text: SharedString,
}

impl Meta {
    pub fn new(icon: impl Into<Icon>, text: impl Into<SharedString>) -> Self {
        Self {
            icon: icon.into(),
            text: text.into(),
        }
    }
}

impl RenderOnce for Meta {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(self.icon.size_4())
            .child(self.text)
    }
}
