use crate::config::Layout;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    SideBySide,
    Stacked,
}

const SIDE_BY_SIDE_MIN_WIDTH: u16 = 80;

pub fn resolve(layout: Layout, viewport_width: u16) -> Resolved {
    match layout {
        Layout::SideBySide => Resolved::SideBySide,
        Layout::Stacked => Resolved::Stacked,
        Layout::Auto => {
            if viewport_width >= SIDE_BY_SIDE_MIN_WIDTH {
                Resolved::SideBySide
            } else {
                Resolved::Stacked
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_picks_side_by_side_when_wide() {
        assert_eq!(resolve(Layout::Auto, 100), Resolved::SideBySide);
    }

    #[test]
    fn auto_picks_stacked_when_narrow() {
        assert_eq!(resolve(Layout::Auto, 60), Resolved::Stacked);
    }

    #[test]
    fn explicit_overrides_width() {
        assert_eq!(resolve(Layout::SideBySide, 30), Resolved::SideBySide);
        assert_eq!(resolve(Layout::Stacked, 200), Resolved::Stacked);
    }
}
