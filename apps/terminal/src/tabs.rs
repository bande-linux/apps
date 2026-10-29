use unbound_toolkit::{Point, Rect};

pub const TAB_BAR_HEIGHT: f32 = 28.0;
const NEW_TAB_WIDTH: f32 = 32.0;
const CLOSE_SIZE: f32 = 16.0;
const CLOSE_INSET: f32 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabTarget {
    Select(usize),
    Close(usize),
    NewTab,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabBarLayout {
    pub tabs: Vec<Rect>,
    pub new_tab: Rect,
}

fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x
        && point.y >= rect.y
        && point.x < rect.x + rect.width
        && point.y < rect.y + rect.height
}

impl TabBarLayout {
    pub fn new(count: usize, width: f32) -> Self {
        let available = (width - NEW_TAB_WIDTH).max(0.0);
        let tab_width = available / count.max(1) as f32;
        let tabs = (0..count)
            .map(|index| {
                let left = (index as f32 * tab_width).round();
                let right = ((index + 1) as f32 * tab_width).round();
                Rect::new(left, 0.0, right - left, TAB_BAR_HEIGHT)
            })
            .collect();
        Self {
            tabs,
            new_tab: Rect::new(available, 0.0, NEW_TAB_WIDTH, TAB_BAR_HEIGHT),
        }
    }

    pub fn close_button(tab: Rect) -> Rect {
        Rect::new(
            tab.x + CLOSE_INSET,
            tab.y + (tab.height - CLOSE_SIZE) / 2.0,
            CLOSE_SIZE,
            CLOSE_SIZE,
        )
    }

    pub fn hit(&self, point: Point) -> Option<TabTarget> {
        if contains(self.new_tab, point) {
            return Some(TabTarget::NewTab);
        }
        let index = self.tabs.iter().position(|tab| contains(*tab, point))?;
        if contains(Self::close_button(self.tabs[index]), point) {
            Some(TabTarget::Close(index))
        } else {
            Some(TabTarget::Select(index))
        }
    }
}
