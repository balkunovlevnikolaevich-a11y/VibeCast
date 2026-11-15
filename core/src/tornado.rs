//! Pure tornado simulation: it never reads or changes real Explorer icons.
use crate::geometry::V2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconState { Resting, Captured, Thrown, Landed }

#[derive(Clone, Debug)]
pub struct Icon {
    pub original: V2,
    pub position: V2,
    pub velocity: V2,
    pub state: IconState,
    age: f32,
    capture_start: V2,
    capture_angle: f32,
}

#[derive(Clone, Debug)]
pub struct Tornado {
    pub center: V2,
    pub icons: Vec<Icon>,
    pub width: f32,
    pub height: f32,
    direction: f32,
    elapsed: f32,
}

impl Tornado {
    pub fn new(width: f32, height: f32, positions: &[V2]) -> Option<Self> {
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0
            || positions.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) { return None; }
        Some(Self { center: V2::new(0.0, height * 0.5), width, height, direction: 1.0, elapsed: 0.0,
            icons: positions.iter().map(|&position| Icon { original: position, position, velocity: V2::ZERO, state: IconState::Resting, age: 0.0, capture_start: position, capture_angle: 0.0 }).collect() })
    }

    pub fn step(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 { return; }
        let dt = dt.min(0.05);
        self.elapsed += dt;
        self.center.x += self.direction * 70.0 * dt;
        if self.center.x >= self.width { self.center.x = self.width; self.direction = -1.0; }
        if self.center.x <= 0.0 { self.center.x = 0.0; self.direction = 1.0; }
        // A funnel can hold only one icon: choose the closest eligible one.
        let candidate = if self.icons.iter().any(|icon| icon.state == IconState::Captured) {
            None
        } else {
            self.icons.iter().enumerate()
                .filter(|(_, icon)| icon.state == IconState::Resting && (icon.position.x - self.center.x).abs() < 70.0)
                .min_by(|(_, a), (_, b)| {
                    let distance = |icon: &Icon| {
                        (icon.position.x - self.center.x).powi(2) + (icon.position.y - self.center.y).powi(2)
                    };
                    distance(a).total_cmp(&distance(b))
                })
                .map(|(index, _)| index)
        };
        for (i, icon) in self.icons.iter_mut().enumerate() {
            match icon.state {
                IconState::Resting if candidate == Some(i) => {
                    icon.state = IconState::Captured; icon.age = 0.0;
                    icon.capture_start = icon.position;
                    icon.capture_angle = (icon.position.y - self.center.y).atan2(icon.position.x - self.center.x);
                }
                IconState::Captured => {
                    icon.age += dt;
                    // Ease each icon into a wide, visible orbit rather than teleporting it
                    // to the middle of the screen. Several turns precede the throw.
                    let a = icon.capture_angle + icon.age * 4.5;
                    let radius = 70.0 + (i % 4) as f32 * 12.0;
                    let orbit = V2::new(self.center.x + a.cos() * radius,
                        self.center.y + a.sin() * radius * 1.35);
                    let blend = (icon.age / 0.9).min(1.0);
                    let blend = blend * blend * (3.0 - 2.0 * blend);
                    icon.position = V2::new((icon.capture_start.x + (orbit.x - icon.capture_start.x) * blend).clamp(0.0, self.width),
                        (icon.capture_start.y + (orbit.y - icon.capture_start.y) * blend).clamp(0.0, self.height));
                    if icon.age >= 4.0 + (i % 4) as f32 * 0.35 {
                        let target = if self.center.x < self.width * 0.5 { self.width } else { 0.0 };
                        icon.velocity = V2::new((target - icon.position.x) * 0.65, -90.0);
                        icon.state = IconState::Thrown;
                    }
                }
                IconState::Thrown => {
                    icon.position += icon.velocity * dt;
                    icon.velocity.y += 190.0 * dt;
                    if icon.position.y >= self.height || icon.position.x <= 0.0 || icon.position.x >= self.width {
                        icon.position.x = icon.position.x.clamp(0.0, self.width);
                        icon.position.y = icon.position.y.clamp(0.0, self.height);
                        icon.state = IconState::Landed;
                    }
                }
                _ => {}
            }
        }
    }

    /// Snapshot only; OS integration must map stable Shell identifiers independently.
    pub fn originals(&self) -> impl Iterator<Item = V2> + '_ { self.icons.iter().map(|icon| icon.original) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captures_throws_and_preserves_originals() {
        let start = [V2::new(20.0, 50.0), V2::new(170.0, 55.0)];
        let mut tornado = Tornado::new(200.0, 100.0, &start).unwrap();
        let mut captured = false; let mut thrown = false;
        for _ in 0..1_000 {
            tornado.step(1.0 / 60.0);
            captured |= tornado.icons.iter().any(|i| i.state == IconState::Captured);
            thrown |= tornado.icons.iter().any(|i| matches!(i.state, IconState::Thrown | IconState::Landed));
            assert!(tornado.icons.iter().all(|i| i.position.x.is_finite() && i.position.y.is_finite()));
        }
        assert!(captured && thrown);
        assert_eq!(tornado.originals().collect::<Vec<_>>(), start);
        assert!(Tornado::new(f32::NAN, 100.0, &start).is_none());
    }

    #[test]
    fn capture_is_smooth_and_orbits_before_throw() {
        let mut tornado = Tornado::new(1200.0, 800.0, &[V2::new(30.0, 80.0)]).unwrap();
        tornado.step(0.05);
        let first = tornado.icons[0].position;
        tornado.step(0.05);
        assert!((tornado.icons[0].position.y - first.y).abs() < 10.0);
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        for _ in 0..60 {
            tornado.step(0.05);
            assert_eq!(tornado.icons[0].state, IconState::Captured);
            min_y = min_y.min(tornado.icons[0].position.y);
            max_y = max_y.max(tornado.icons[0].position.y);
        }
        assert!(max_y - min_y > 120.0, "icon must visibly orbit the funnel");
    }

    #[test]
    fn captures_only_one_icon_at_a_time_then_moves_to_the_next() {
        let positions = [V2::new(30.0, 80.0), V2::new(30.0, 100.0), V2::new(30.0, 120.0)];
        let mut tornado = Tornado::new(200.0, 180.0, &positions).unwrap();
        for _ in 0..600 {
            tornado.step(0.05);
            assert!(tornado.icons.iter().filter(|icon| icon.state == IconState::Captured).count() <= 1);
        }
        assert!(tornado.icons.iter().all(|icon| icon.state != IconState::Resting));
        assert_eq!(tornado.originals().collect::<Vec<_>>(), positions);
    }
}
