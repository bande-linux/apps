use unbound_toolkit::{HEADER_HEIGHT, Point, Rect};

pub const STRIP_HEIGHT: f32 = HEADER_HEIGHT as f32;
pub const TAB_HEIGHT: f32 = 28.0;
pub const HEADER_LEADING: f32 = 94.0;
pub const FULL_SCREEN_LEADING: f32 = 12.0;
const TRAILING: f32 = 12.0;
const NEW_TAB_SIZE: f32 = 28.0;
const NEW_TAB_GAP: f32 = 8.0;
const TAB_GAP: f32 = 4.0;
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
    pub fn new(count: usize, width: f32, leading: f32) -> Self {
        let top = ((STRIP_HEIGHT - TAB_HEIGHT) / 2.0).round();
        let new_tab_x = (width - TRAILING - NEW_TAB_SIZE).max(leading);
        let new_tab = Rect::new(new_tab_x, top, NEW_TAB_SIZE, NEW_TAB_SIZE);
        let shown = if count > 1 { count } else { 0 };
        let available = (new_tab_x - NEW_TAB_GAP - leading).max(0.0);
        let pitch = (available + TAB_GAP) / shown.max(1) as f32;
        let tabs = (0..shown)
            .map(|index| {
                let left = (leading + index as f32 * pitch).round();
                let right = (leading + (index + 1) as f32 * pitch - TAB_GAP).round();
                Rect::new(left, top, (right - left).max(0.0), TAB_HEIGHT)
            })
            .collect();
        Self { tabs, new_tab }
    }

    pub fn items(&self) -> Vec<Rect> {
        self.tabs.iter().copied().chain([self.new_tab]).collect()
    }

    pub fn close_button(tab: Rect) -> Rect {
        Rect::new(
            tab.x + CLOSE_INSET,
            tab.y + ((tab.height - CLOSE_SIZE) / 2.0).round(),
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
