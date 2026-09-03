#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Horizontal,
            Self::Up | Self::Down => Axis::Vertical,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    fn center(self) -> (f32, f32) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Pane(usize),
    Split {
        axis: Axis,
        ratio: f32,
        first: Box<Node>,
        second: Box<Node>,
    },
}

impl Node {
    pub fn pane(id: usize) -> Self {
        Self::Pane(id)
    }

    pub fn split(&mut self, target: usize, new_id: usize, axis: Axis) -> bool {
        self.split_at(target, new_id, axis, false)
    }

    pub fn split_at(&mut self, target: usize, new_id: usize, axis: Axis, before: bool) -> bool {
        match self {
            Self::Pane(id) if *id == target => {
                let target = Self::Pane(target);
                let new = Self::Pane(new_id);
                let (first, second) = if before { (new, target) } else { (target, new) };
                *self = Self::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(first),
                    second: Box::new(second),
                };
                true
            }
            Self::Pane(_) => false,
            Self::Split { first, second, .. } => {
                first.split_at(target, new_id, axis, before)
                    || second.split_at(target, new_id, axis, before)
            }
        }
    }

    pub fn contains(&self, target: usize) -> bool {
        match self {
            Self::Pane(id) => *id == target,
            Self::Split { first, second, .. } => first.contains(target) || second.contains(target),
        }
    }

    pub fn remove(self, target: usize) -> Option<Self> {
        match self {
            Self::Pane(id) => (id != target).then_some(Self::Pane(id)),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => match (first.remove(target), second.remove(target)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    axis,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(node), None) | (None, Some(node)) => Some(node),
                (None, None) => None,
            },
        }
    }

    pub fn pane_ids(&self) -> Vec<usize> {
        let mut ids = Vec::new();
        self.collect_ids(&mut ids);
        ids
    }

    fn collect_ids(&self, ids: &mut Vec<usize>) {
        match self {
            Self::Pane(id) => ids.push(*id),
            Self::Split { first, second, .. } => {
                first.collect_ids(ids);
                second.collect_ids(ids);
            }
        }
    }

    pub fn rects(&self) -> Vec<(usize, Rect)> {
        let mut rects = Vec::new();
        self.collect_rects(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            &mut rects,
        );
        rects
    }

    fn collect_rects(&self, rect: Rect, out: &mut Vec<(usize, Rect)>) {
        match self {
            Self::Pane(id) => out.push((*id, rect)),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (a, b) = match axis {
                    Axis::Horizontal => (
                        Rect {
                            width: rect.width * ratio,
                            ..rect
                        },
                        Rect {
                            x: rect.x + rect.width * ratio,
                            width: rect.width * (1.0 - ratio),
                            ..rect
                        },
                    ),
                    Axis::Vertical => (
                        Rect {
                            height: rect.height * ratio,
                            ..rect
                        },
                        Rect {
                            y: rect.y + rect.height * ratio,
                            height: rect.height * (1.0 - ratio),
                            ..rect
                        },
                    ),
                };
                first.collect_rects(a, out);
                second.collect_rects(b, out);
            }
        }
    }

    pub fn neighbor(&self, focused: usize, direction: Direction) -> Option<usize> {
        let rects = self.rects();
        let current = rects.iter().find(|(id, _)| *id == focused)?.1;
        let (cx, cy) = current.center();

        rects
            .into_iter()
            .filter(|(id, _)| *id != focused)
            .filter_map(|(id, rect)| {
                let (x, y) = rect.center();
                let (primary, secondary) = match direction {
                    Direction::Left if x < cx => (cx - x, (cy - y).abs()),
                    Direction::Right if x > cx => (x - cx, (cy - y).abs()),
                    Direction::Up if y < cy => (cy - y, (cx - x).abs()),
                    Direction::Down if y > cy => (y - cy, (cx - x).abs()),
                    _ => return None,
                };
                Some((id, primary + secondary * 2.0))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }

    pub fn swap_panes(&mut self, a: usize, b: usize) {
        self.replace_id(a, usize::MAX);
        self.replace_id(b, a);
        self.replace_id(usize::MAX, b);
    }

    fn replace_id(&mut self, old: usize, new: usize) -> bool {
        match self {
            Self::Pane(id) if *id == old => {
                *id = new;
                true
            }
            Self::Pane(_) => false,
            Self::Split { first, second, .. } => {
                first.replace_id(old, new) || second.replace_id(old, new)
            }
        }
    }

    pub fn resize(&mut self, focused: usize, direction: Direction, amount: f32) -> bool {
        match self {
            Self::Pane(_) => false,
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                if *axis == direction.axis()
                    && (first.contains(focused) || second.contains(focused))
                {
                    let focused_first = first.contains(focused);
                    let toward_second = matches!(direction, Direction::Right | Direction::Down);
                    let delta = if focused_first == toward_second {
                        amount
                    } else {
                        -amount
                    };
                    *ratio = (*ratio + delta).clamp(0.2, 0.8);
                    true
                } else if first.contains(focused) {
                    first.resize(focused, direction, amount)
                } else {
                    second.resize(focused, direction, amount)
                }
            }
        }
    }

    /// Moves the nearest matching split divider by a delta expressed as a
    /// fraction of the root layout. The divider follows the pointer regardless
    /// of which side contains the focused pane.
    pub fn resize_from_pointer(&mut self, focused: usize, axis: Axis, delta: f32) -> bool {
        self.resize_from_pointer_in_span(focused, axis, delta, 1.0)
    }

    fn resize_from_pointer_in_span(
        &mut self,
        focused: usize,
        target_axis: Axis,
        delta: f32,
        span: f32,
    ) -> bool {
        match self {
            Self::Pane(_) => false,
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let focused_first = first.contains(focused);
                let focused_second = second.contains(focused);
                if !focused_first && !focused_second {
                    return false;
                }

                let child_span = if *axis == target_axis {
                    span * if focused_first { *ratio } else { 1.0 - *ratio }
                } else {
                    span
                };
                let child_resized = if focused_first {
                    first.resize_from_pointer_in_span(focused, target_axis, delta, child_span)
                } else {
                    second.resize_from_pointer_in_span(focused, target_axis, delta, child_span)
                };

                if child_resized {
                    true
                } else if *axis == target_axis {
                    *ratio = (*ratio + delta / span.max(f32::EPSILON)).clamp(0.2, 0.8);
                    true
                } else {
                    false
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Node {
        Node::Split {
            axis: Axis::Horizontal,
            ratio: 0.5,
            first: Box::new(Node::pane(1)),
            second: Box::new(Node::Split {
                axis: Axis::Vertical,
                ratio: 0.5,
                first: Box::new(Node::pane(2)),
                second: Box::new(Node::pane(3)),
            }),
        }
    }

    #[test]
    fn splits_and_collapses_tree() {
        let mut root = Node::pane(1);
        assert!(root.split(1, 2, Axis::Horizontal));
        assert!(root.split(2, 3, Axis::Vertical));
        assert_eq!(root.pane_ids(), vec![1, 2, 3]);

        root = root.remove(2).unwrap();
        assert_eq!(root.pane_ids(), vec![1, 3]);
    }

    #[test]
    fn inserts_before_or_after_drop_target() {
        let mut before = Node::pane(1);
        assert!(before.split_at(1, 2, Axis::Horizontal, true));
        assert_eq!(before.pane_ids(), vec![2, 1]);

        let mut after = Node::pane(1);
        assert!(after.split_at(1, 2, Axis::Vertical, false));
        assert_eq!(after.pane_ids(), vec![1, 2]);
    }

    #[test]
    fn finds_spatial_neighbors() {
        let root = sample();
        assert_eq!(root.neighbor(1, Direction::Right), Some(2));
        assert_eq!(root.neighbor(2, Direction::Down), Some(3));
        assert_eq!(root.neighbor(3, Direction::Left), Some(1));
        assert_eq!(root.neighbor(1, Direction::Left), None);
    }

    #[test]
    fn directional_swap_moves_both_panes_without_changing_identity() {
        let mut root = sample();
        let neighbor = root.neighbor(1, Direction::Right).unwrap();

        root.swap_panes(1, neighbor);

        let rects = root.rects();
        let focused = rects.iter().find(|(id, _)| *id == 1).unwrap().1;
        let swapped = rects.iter().find(|(id, _)| *id == neighbor).unwrap().1;
        assert_eq!(focused.x, 0.5);
        assert_eq!(focused.y, 0.0);
        assert_eq!(swapped.x, 0.0);
        assert_eq!(swapped.y, 0.0);
        assert_eq!(root.pane_ids().len(), 3);
    }

    #[test]
    fn resize_is_bounded() {
        let mut root = sample();
        for _ in 0..20 {
            root.resize(1, Direction::Right, 0.05);
        }
        match root {
            Node::Split { ratio, .. } => assert_eq!(ratio, 0.8),
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn pointer_resize_follows_divider_for_every_tile_side() {
        let mut lower = sample();
        assert!(lower.resize_from_pointer(3, Axis::Vertical, -0.1));
        let lower_height = lower
            .rects()
            .into_iter()
            .find(|(id, _)| *id == 3)
            .unwrap()
            .1
            .height;
        assert!((lower_height - 0.6).abs() < f32::EPSILON);

        let mut right = sample();
        assert!(right.resize_from_pointer(2, Axis::Horizontal, -0.1));
        let right_width = right
            .rects()
            .into_iter()
            .find(|(id, _)| *id == 2)
            .unwrap()
            .1
            .width;
        assert!((right_width - 0.6).abs() < f32::EPSILON);

        let mut upper = sample();
        assert!(upper.resize_from_pointer(2, Axis::Vertical, 0.1));
        let upper_height = upper
            .rects()
            .into_iter()
            .find(|(id, _)| *id == 2)
            .unwrap()
            .1
            .height;
        assert!((upper_height - 0.6).abs() < f32::EPSILON);
    }
}
