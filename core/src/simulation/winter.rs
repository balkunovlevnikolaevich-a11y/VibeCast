use crate::geometry::V2;

#[derive(Clone, Copy)]
pub(super) struct WinterBall {
    pub x: f32,
    pub y: f32,
    pub velocity: V2,
    pub diameter: f32,
    pub layer_id: isize,
}



pub(super) struct SnowSplat {
    pub position: V2,
    pub offset: V2,
    pub window: Option<isize>,
    pub frame: [f32; 4],
    pub size: f32,
    pub age: f32,
    pub duration: f32,
}

/// A small, bounded snow clump thrown off the bottom of the desktop by the snowman.
#[derive(Clone, Copy)]
pub(super) struct PanelSnow {
    pub position: V2,
    pub velocity: V2,
    pub size: f32,
    /// Share of the ground snow area represented by this clump.
    pub area: f32,
}

#[derive(Default)]
pub(super) struct Winter {
    pub rolling: bool,
    pub working: Option<WinterBall>,
    pub balls: Vec<WinterBall>,
    pub tractor_x: f32,
    pub tractor_direction: f32,
    pub tractor_wait: f32,
    pub tractor_interval: f32,
    pub tractor_active: bool,
    pub tractor_snow_area: f32,
    pub plow_disposal_clock: f64,
    pub initialized: bool,
    pub cursor: V2,
    pub cursor_known: bool,
    pub pointer_velocity: V2,
    pub held_new: bool,
    pub cleaner: u32,
    pub garlands: bool,
    pub scene: u32,
    pub image: Option<String>,
    pub window_walls: bool,
    pub splat_seconds: u32,
    pub splats: Vec<SnowSplat>,
    pub plow_cut: Vec<bool>,
    pub plow_track: Vec<(f32, f32, f32)>,
    pub plow_y: f32,
    pub plow_angle: f32,
    pub plow_last_track_x: f32,
    /// Normalized six-beat animation cycle for the animated snowman cleaner.
    pub snowman_phase: f32,
    /// Snow clumps thrown toward and through the taskbar, capped for low-end machines.
    pub panel_snow: Vec<PanelSnow>,
    /// Visible taskbar depth in DIP; zero makes thrown snow leave at the screen edge.
    pub panel_height: f32,
}

fn snow_group(groups: &mut [usize], mut index: usize) -> usize {
    while groups[index] != index {
        groups[index] = groups[groups[index]];
        index = groups[index];
    }
    index
}

use super::{SnowSimulation, Sprite};
use crate::layer::Blend;

impl SnowSimulation {
    pub(super) fn plow_ball_radius(&self) -> f32 {
        (self.winter.tractor_snow_area * 1.5).cbrt().min(80.0)
    }
    pub fn set_winter_cleaner(&mut self, value: u32) { self.winter.cleaner = value.min(6); }
    pub fn set_winter_window_walls(&mut self, value: bool) { self.winter.window_walls = value; }

    pub fn set_winter_splat_seconds(&mut self, seconds: u32) {
        self.winter.splat_seconds = seconds.min(3600);
        for splat in &mut self.winter.splats { splat.duration = seconds.min(3600) as f32; }
    }
    pub fn set_winter_interval(&mut self, seconds: u32) {
        self.winter.tractor_interval = seconds.min(3600) as f32;
        if self.winter.initialized && !self.winter.tractor_active {
            self.winter.tractor_wait = self.winter.tractor_interval;
        }
    }
    pub fn set_winter_garlands(&mut self, value: bool) { self.winter.garlands = value; }
    pub fn set_winter_scene(&mut self, value: u32) { self.winter.scene = value.min(2); }
    pub fn winter_scene(&self) -> u32 { self.winter.scene }
    pub fn set_winter_wind(&mut self, value: f32) { self.air.manual_wind = value.clamp(-1.0, 1.0); }
    pub fn set_winter_image(&mut self, value: Option<String>) { self.winter.image = value; }
    pub fn winter_image(&self) -> Option<&str> { self.winter.image.as_deref() }
    pub(super) fn winter_balls_step(&mut self, dt: f32, cursor: V2, holding: bool) -> bool {
        let mut changed = !self.winter.splats.is_empty() || !self.winter.balls.is_empty() || self.winter.working.is_some() || !self.winter.plow_track.is_empty();
        for splat in &mut self.winter.splats { splat.age += dt; }
        self.winter.splats.retain(|s| s.age < s.duration);
        for track in &mut self.winter.plow_track { track.2 += dt; }
        self.winter.plow_track.retain(|t| t.2 < 12.0);
        let velocity = if self.winter.cursor_known { (cursor - self.winter.cursor) / dt.max(0.004) } else { V2::ZERO };
        let speed = velocity.length();
        let velocity = if speed > 2400.0 { velocity * (2400.0 / speed) } else { velocity };
        self.winter.pointer_velocity = self.winter.pointer_velocity * 0.45 + velocity * 0.55;
        self.winter.cursor = cursor;
        self.winter.cursor_known = true;
        if holding {
            if self.winter.working.is_none() {
                let hit = self.winter.balls.iter().rposition(|b| (V2::new(b.x, b.y) - cursor).length() < b.diameter * 0.46 + 10.0);
                if let Some(index) = hit {
                    self.winter.working = Some(self.winter.balls.remove(index));
                    self.winter.held_new = false;
                } else {
                    let found = self.layers.iter().enumerate().find_map(|(li, layer)| {
                        let i = layer.index(cursor.x)?;
                        (layer.visible[i] && layer.heights[i] > 0.4 && (cursor.y - layer.surface(i)).abs() < 18.0).then_some(self.ids[li])
                    });
                    if let Some(layer_id) = found {
                        self.winter.working = Some(WinterBall { x: cursor.x, y: cursor.y, diameter: 6.0, velocity: V2::ZERO, layer_id });
                        self.winter.held_new = true;
                    }
                }
            }
            if let Some(mut ball) = self.winter.working.take() {
                ball.x = cursor.x.clamp(ball.diameter * 0.46, (self.width - ball.diameter * 0.46).max(ball.diameter * 0.46));
                ball.y = cursor.y.max(ball.diameter * 0.46);
                ball.velocity = self.winter.pointer_velocity;
                if let Some(li) = self.ids.iter().position(|&id| id == ball.layer_id) {
                    let layer = &mut self.layers[li];
                    if let Some(i) = layer.index(ball.x) {
                        if (cursor.y - layer.surface(i)).abs() < 24.0 {
                            // Grow from real snow removed under the entire rolling footprint.
                            let radius = (ball.diameter * 0.46).max(0.1);
                            let footprint = radius.max(10.0);
                            let mut remaining = (std::f32::consts::PI * ((44.0_f32 * 0.46).powi(2) - radius.powi(2))).max(0.0);
                            let mut removed = 0.0;
                            let first = layer.clamped_index(ball.x - footprint);
                            let last = layer.clamped_index(ball.x + footprint);
                            for column in first..=last {
                                let weight = (1.0 - (layer.center_x(column) - ball.x).abs() / (footprint + super::COLUMN_WIDTH)).clamp(0.0, 1.0);
                                let take = (dt * 28.0 * weight).min(layer.heights[column]).min(remaining / super::COLUMN_WIDTH);
                                layer.set(column, layer.heights[column] - take);
                                let area = take * super::COLUMN_WIDTH;
                                remaining -= area;
                                removed += area;
                            }
                            ball.diameter = ((radius * radius + removed / std::f32::consts::PI).sqrt() / 0.46).min(44.0);
                        }
                    }
                }
                self.winter.working = Some(ball);
                changed = true;
            }
        } else if let Some(mut ball) = self.winter.working.take() {
            // A gentle placement near a stack lands on its top; a throw keeps its velocity.
            if self.winter.held_new && ball.velocity.length() < 180.0 {
                if let Some(top) = self.winter.balls.iter().filter(|other| (other.x - ball.x).abs() < 42.0).max_by(|a,b| a.y.total_cmp(&b.y)) {
                    ball.x = top.x;
                    ball.y = top.y + (top.diameter + ball.diameter) * 0.46;
                    ball.velocity = V2::ZERO;
                }
            }
            ball.layer_id = -1;
            if self.winter.balls.len() < 48 { self.winter.balls.push(ball); }
            changed = true;
        }
        self.winter.rolling = holding && self.winter.working.is_some();
        let mut balls = std::mem::take(&mut self.winter.balls);
        let mut broken = vec![false; balls.len()];
        // Small time steps keep a fast throw from tunnelling through another ball.
        let fastest = balls.iter().map(|b| b.velocity.length()).fold(0.0_f32, f32::max);
        let smallest = balls.iter().map(|b| b.diameter * 0.46).fold(20.0_f32, f32::min).max(2.0);
        let steps = (((fastest + 900.0 * dt) * dt / smallest).ceil() as usize).clamp(1, 32);
        let frame_dt = dt;
        for _ in 0..steps {
        let dt = frame_dt / steps as f32;
        for (index, ball) in balls.iter_mut().enumerate() {
            if broken[index] { continue; }
            let radius = ball.diameter * 0.46;
            let previous_bottom = ball.y - radius;
            ball.velocity.y -= 900.0 * dt;
            ball.velocity.x *= (-1.4 * dt).exp();
            ball.x += ball.velocity.x * dt;
            ball.y += ball.velocity.y * dt;
            if self.winter.window_walls && ball.velocity.length() > 120.0 {
                let impact = self.covers.iter().copied().find(|rect| {
                    let near = V2::new(ball.x.clamp(rect[0], rect[2]), ball.y.clamp(rect[1], rect[3]));
                    (V2::new(ball.x, ball.y) - near).length() < radius - 0.1
                });
                if let Some(rect) = impact {
                    broken[index] = true;
                    if self.winter.splat_seconds > 0 {
                        let position = V2::new(ball.x.clamp(rect[0], rect[2]), ball.y.clamp(rect[1], rect[3]));
                        let window = self.layers.iter().enumerate().skip(1).find(|(_, layer)| {
                            (layer.base().x - rect[0]).abs() < 12.0 && (layer.base().y - rect[3]).abs() < 12.0
                        }).map(|(li, layer)| (self.ids[li], layer.base()));
                        if self.winter.splats.len() >= 80 { self.winter.splats.remove(0); }
                        self.winter.splats.push(SnowSplat { position, offset: window.map_or(V2::ZERO, |(_, origin)| position - origin), window: window.map(|(id,_)| id), frame: rect, size: ball.diameter * 1.8, age: 0.0, duration: self.winter.splat_seconds as f32 });
                    }

                    continue;
                }
            }
            if ball.x < radius || ball.x > self.width - radius {
                if ball.velocity.x.abs() > 650.0 { broken[index] = true; }
                ball.x = ball.x.clamp(radius, (self.width - radius).max(radius));
                ball.velocity.x *= -0.3;
            }
            let mut floor: f32 = 0.0;
            let mut support = 0;
            for (li, layer) in self.layers.iter().enumerate() {
                if li > 0 && !self.winter.window_walls { continue; }
                if let Some(i) = layer.index(ball.x) {
                    let surface = layer.surface(i);
                    if layer.visible[i] && surface <= previous_bottom + 2.0 && surface > floor {
                        floor = surface;
                        support = self.ids[li];
                    }
                }
            }
            if ball.y - radius <= floor && ball.velocity.y <= 0.0 {
                if ball.velocity.length() > 650.0 { broken[index] = true; }
                ball.y = floor + radius;
                ball.velocity.y = if ball.velocity.y < -90.0 { -ball.velocity.y * 0.22 } else { 0.0 };
                ball.velocity.x *= (-6.0 * dt).exp();
                ball.layer_id = support;
            } else { ball.layer_id = -1; }
        }
        // Resolve circle contacts. The lower ball supports an upper ball at rest.
        for _ in 0..4 {
            for a in 0..balls.len() {
                for b in a + 1..balls.len() {
                    if broken[a] || broken[b] { continue; }
                    let delta = V2::new(balls[b].x - balls[a].x, balls[b].y - balls[a].y);
                    let reach = (balls[a].diameter + balls[b].diameter) * 0.46;
                    if delta.x.abs() >= reach || delta.y.abs() >= reach { continue; }
                    let distance = delta.length();
                    if distance >= reach { continue; }
                    let normal = if distance > 0.001 { delta / distance } else { V2::new(0.0, 1.0) };
                    let relative = (balls[b].velocity - balls[a].velocity).dot(normal);
                    if relative < -650.0 { broken[a] = true; broken[b] = true; continue; }
                    let overlap = reach - distance;
                    if normal.y > 0.65 && balls[a].layer_id >= 0 {
                        balls[b].x += normal.x * overlap;
                        balls[b].y += normal.y * overlap;
                        if balls[b].velocity.y < 0.0 { balls[b].velocity.y = 0.0; }
                        balls[b].layer_id = balls[a].layer_id;
                    } else if normal.y < -0.65 && balls[b].layer_id >= 0 {
                        balls[a].x -= normal.x * overlap;
                        balls[a].y -= normal.y * overlap;
                        if balls[a].velocity.y < 0.0 { balls[a].velocity.y = 0.0; }
                        balls[a].layer_id = balls[b].layer_id;
                    } else {
                        let inverse_a = 1.0 / balls[a].diameter.powi(2);
                        let inverse_b = 1.0 / balls[b].diameter.powi(2);
                        let total = inverse_a + inverse_b;
                        balls[a].x -= normal.x * overlap * inverse_a / total;
                        balls[a].y -= normal.y * overlap * inverse_a / total;
                        balls[b].x += normal.x * overlap * inverse_b / total;
                        balls[b].y += normal.y * overlap * inverse_b / total;
                        if relative < 0.0 {
                            let impulse = normal * (-relative * 1.12 / total);
                            balls[a].velocity -= impulse * inverse_a;
                            balls[b].velocity += impulse * inverse_b;
                        }
                    }
                }
            }
        }
        }
        for (index, ball) in balls.into_iter().enumerate() {
            if broken[index] {
                let radius = ball.diameter * 0.46;
                self.air.release(radius * radius * 1.2, Blend::SNOW, 0.0, V2::new(ball.x, ball.y), ball.velocity * 0.25 + V2::new(0.0, 100.0), V2::new(radius, radius), 1.0);
            } else {
                let mut ball = ball;
                if let Some(li) = self.ids.iter().position(|id| *id == ball.layer_id) {
                    if let Some(i) = self.layers[li].index(ball.x) {
                        let floor = self.layers[li].surface(i) + ball.diameter * 0.46;
                        if ball.y < floor { ball.y = floor; ball.velocity.y = ball.velocity.y.max(0.0); }
                    }
                }
                self.winter.balls.push(ball);
            }
        }
        changed
    }

    pub(super) fn winter_ball_sprites(&self) -> Vec<Sprite> {
        let mut out = Vec::new();

        for splat in &self.winter.splats {
            let (position, frame) = if let Some(id) = splat.window {
                let Some(li) = self.ids.iter().position(|other| *other == id) else { continue };
                let layer = &self.layers[li];
                let origin = layer.base();
                (origin + splat.offset, [origin.x, origin.y - layer.face, origin.x + layer.width(), origin.y])
            } else {
                if !self.covers.contains(&splat.frame) { continue; }
                (splat.position, splat.frame)
            };
            let size = splat.size;
            let alpha = ((splat.duration - splat.age) / (splat.duration * 0.3).max(0.1)).clamp(0.0, 1.0);
            out.push(Sprite { position, size, alpha, ghost: [-6.0, 0.0, 0.0, 0.0], pose: [(frame[0]-position.x)/size, (frame[1]-position.y)/size, (frame[2]-position.x)/size, (frame[3]-position.y)/size], ..Sprite::default() });
        }
        if self.winter.garlands {
            for layer in self.layers.iter().skip(1) {
                for k in 0..((layer.width() / 32.0) as usize).min(100) {
                    let x = layer.base().x + 16.0 + k as f32 * 32.0;
                    let i = layer.clamped_index(x);
                    if layer.visible[i] {
                        out.push(Sprite { position: V2::new(x, layer.floor(i) - 9.0), size: 34.0, alpha: 1.0, ghost: [-4.0, k as f32, 0.0, 0.0], ..Sprite::default() });
                    }
                    if out.len() >= 300 { break; }
                }
                if out.len() >= 300 { break; }
            }
        }
        let balls = &self.winter.balls;
        let mut groups: Vec<usize> = (0..balls.len()).collect();
        for a in 0..balls.len() {
            for b in a+1..balls.len() {
                let dx = (balls[a].x - balls[b].x).abs();
                let dy = (balls[a].y - balls[b].y).abs();
                let reach = (balls[a].diameter + balls[b].diameter) * 0.46;
                if dx < balls[a].diameter.min(balls[b].diameter) * 0.45 && dy > reach * 0.4 && dx*dx + dy*dy <= (reach + 3.0).powi(2) {
                    let left = snow_group(&mut groups, a);
                    let right = snow_group(&mut groups, b);
                    groups[right] = left;
                }
            }
        }
        let mut counts = vec![0; balls.len()];
        let mut tops: Vec<Option<usize>> = vec![None; balls.len()];
        for i in 0..balls.len() {
            let group = snow_group(&mut groups, i);
            counts[group] += 1;
            if tops[group].is_none_or(|j| balls[i].y > balls[j].y) { tops[group] = Some(i); }
        }
        for (index, ball) in balls.iter().enumerate() {
            let group = snow_group(&mut groups, index);
            let head = counts[group] >= 3 && tops[group] == Some(index);
            out.push(Sprite { position: V2::new(ball.x, ball.y), size: ball.diameter, alpha: 1.0, ghost: [if head { -3.0 } else { -2.0 }, 0.0, 0.0, 0.0], ..Sprite::default() });
        }
        if let Some(ball) = self.winter.working {
            out.push(Sprite { position: V2::new(ball.x, ball.y), size: ball.diameter, alpha: 1.0, ghost: [-2.0, 0.0, 0.0, 0.0], ..Sprite::default() });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Mode, SnowSimulation};
    use crate::geometry::V2;

    #[test]
    fn rolling_ball_visibly_removes_snow_under_cursor() {
        let mut sim = snowy_ground();
        sim.winter.tractor_wait = 1000.0;
        let before: f32 = sim.layers[0].heights.iter().sum();
        for _ in 0..30 {
            let i = sim.layers[0].clamped_index(400.0);
            let y = sim.layers[0].surface(i);
            sim.winter_step(1.0 / 60.0, V2::new(400.0, y), true);
        }
        let after: f32 = sim.layers[0].heights.iter().sum();
        assert!(before - after > 30.0);
        let diameter = sim.winter.working.unwrap().diameter;
        assert!(diameter > 10.0);
        let ball_gain = std::f32::consts::PI * ((diameter * 0.46).powi(2) - (6.0_f32 * 0.46).powi(2));
        assert!(((before - after) * super::super::COLUMN_WIDTH - ball_gain).abs() < 0.5);
    }

    #[test]
    fn falling_sheet_preserves_profile_instead_of_drawing_round_lumps() {
        let mut sim = snowy_ground();
        for i in 0..sim.layers[0].count() { sim.layers[0].set(i, 10.0 + (i % 24) as f32); }
        let original = sim.layers[0].heights.clone();
        sim.air.shed(&mut sim.layers[0], 1.0, V2::new(200.0, -20.0), false);
        let restored: Vec<f32> = sim.air.grains.iter().flat_map(|g| g.sheet[..g.sheet_count].iter().copied()).collect();
        assert_eq!(restored, original);
        let mut sprites = vec![super::super::Sprite::default(); sim.sprite_count()];
        let drawn = sim.write_sprites(&mut sprites);
        assert_eq!(sprites[..drawn].iter().filter(|s| s.ghost[0] == -7.0).count(), original.len());
        assert!(sprites[..drawn].iter().all(|s| s.lump == 0.0));
    }

    #[test]
    fn splat_ignores_mouse_and_disappears_only_after_its_timer() {
        let mut sim = snowy_ground();
        sim.winter.tractor_wait = 1000.0;
        sim.covers = vec![[0.0, 0.0, 800.0, 600.0]];
        sim.winter.splats.push(super::SnowSplat { position: V2::new(50.0, 40.0), offset: V2::ZERO, window: None, frame: sim.covers[0], size: 40.0, age: 0.0, duration: 1.0 });
        sim.winter_step(0.05, V2::new(-50.0, 40.0), false);
        sim.winter_step(0.05, V2::new(150.0, 40.0), false);
        assert_eq!(sim.winter.splats.len(), 1);
        assert!(sim.winter_ball_sprites().iter().any(|s| s.ghost[0] == -6.0));
        for _ in 0..20 { sim.winter_step(0.05, V2::new(50.0, 40.0), false); }
        assert!(sim.winter.splats.is_empty());
    }

    #[test]
    fn unequal_snowballs_conserve_horizontal_momentum_in_collision() {
        let mut sim = snowy_ground();
        sim.winter.tractor_wait = 1000.0;
        for (x, diameter, vx) in [(300.0, 10.0, 100.0), (320.0, 40.0, -100.0)] {
            sim.winter.balls.push(super::WinterBall { x, y: 300.0, diameter, velocity: V2::new(vx, 0.0), layer_id: -1 });
        }
        let dt: f32 = 1.0 / 60.0;
        let before: f32 = sim.winter.balls.iter().map(|b| b.velocity.x * b.diameter.powi(2)).sum();
        sim.winter_step(dt, V2::new(-1000.0, -1000.0), false);
        let after: f32 = sim.winter.balls.iter().map(|b| b.velocity.x * b.diameter.powi(2)).sum();
        assert!((after - before * (-1.4 * dt).exp()).abs() < 0.2);
        assert!(sim.winter.balls[0].velocity.x < sim.winter.balls[1].velocity.x);
    }

    #[test]
    fn snow_mass_batch_preserves_snow_and_shared_velocity() {
        let mut sim = snowy_ground();
        let area = sim.layers[0].heights.iter().sum::<f32>() * super::super::COLUMN_WIDTH;
        let velocity = V2::new(900.0, -150.0);
        sim.air.shed(&mut sim.layers[0], 1.0, velocity, false);
        assert!(sim.air.grains.len() < 40);
        assert!((sim.air.grains.iter().map(|g| g.area).sum::<f32>() - area).abs() < 0.1);
        assert!(sim.air.grains.iter().all(|g| (g.velocity - velocity).length() < 0.01));
        assert!(sim.layers[0].heights.iter().all(|h| *h == 0.0));
    }

    #[test]
    fn cleaner_tilt_is_smooth_and_limited() {
        let mut sim = snowy_ground();
        for i in 0..sim.layers[0].count() { sim.layers[0].set(i, 60.0 + i as f32 * 0.25); }
        let mut previous = 0.0;
        for _ in 0..90 {
            sim.winter_step(1.0/60.0, V2::new(-1000.0, -1000.0), false);
            let angle = sim.winter.plow_angle;
            assert!(angle.abs() <= 0.16);
            assert!((angle - previous).abs() < 0.02);
            previous = angle;
        }
        assert!(previous > 0.0);
    }

    fn snowy_ground() -> SnowSimulation {
        let mut sim = SnowSimulation::new(800.0, 600.0, 0.0);
        for i in 0..sim.layers[0].count() {
            sim.layers[0].set(i, 35.0);
        }
        sim.layers[0].refresh_top();
        sim
    }

    #[test]
    fn plow_moves_snow_and_stays_above_ground() {
        let mut sim = snowy_ground();
        let i = sim.layers[0].clamped_index(35.0);
        let before = sim.layers[0].heights[i];
        let total_before: f32 = sim.layers[0].heights.iter().sum();
        for _ in 0..90 {
            sim.winter_step(1.0 / 60.0, V2::new(-1000.0, -1000.0), false);
        }
        assert!(sim.layers[0].heights[i] < before - 5.0);
        let total_after: f32 = sim.layers[0].heights.iter().sum::<f32>() + sim.winter.tractor_snow_area / super::super::COLUMN_WIDTH;
        assert!((total_after - total_before).abs() < 0.2, "snow should be pushed, not lost");
        let plow = sim.winter_sprites().into_iter().find(|sprite| sprite.ghost[0] == -1.0).unwrap();
        assert!(plow.position.y > 0.0);
        // A higher remaining drift must lift the vehicle onto its new edge.
        for i in 0..sim.layers[0].count() { sim.layers[0].set(i, 80.0); }
        sim.winter_step(0.05, V2::new(-1000.0, -1000.0), false);
        let raised = sim.winter_sprites().into_iter().find(|sprite| sprite.ghost[0] == -1.0).unwrap();
        assert!(raised.position.y > plow.position.y);
    }

    #[test]
    fn moving_cursor_across_taskbar_does_not_brush_away_snow() {
        let mut sim = snowy_ground();
        let before = sim.layers[0].heights.clone();
        for x in (0..800).step_by(4) {
            sim.track_cursor(V2::new(x as f32, 30.0), 1.0 / 60.0);
        }
        assert_eq!(before, sim.layers[0].heights);
    }

    #[test]
    fn rolling_three_balls_builds_a_snowman_on_the_surface() {
        let mut sim = snowy_ground();
        sim.winter.tractor_wait = 1000.0;
        for _ in 0..3 {
            for i in sim.layers[0].clamped_index(380.0)..=sim.layers[0].clamped_index(420.0) {
                sim.layers[0].set(i, 35.0);
            }
            for _ in 0..60 {
                let i = sim.layers[0].clamped_index(400.0);
                let y = sim.layers[0].surface(i);
                sim.winter_step(1.0 / 60.0, V2::new(400.0, y), true);
            }
            for _ in 0..40 { sim.winter_step(1.0 / 60.0, V2::new(300.0, 60.0), true); }
            sim.winter_step(1.0 / 60.0, V2::new(300.0, 35.0), false);
        }
        let sprites = sim.winter_sprites();
        let balls: Vec<_> = sprites.iter().filter(|sprite| sprite.ghost[0] <= -2.0 && sprite.ghost[3] == 0.0).collect();
        assert_eq!(balls.len(), 3);
        assert_eq!(balls[2].ghost[0], -3.0);
        assert!(balls[0].position.y < balls[1].position.y && balls[1].position.y < balls[2].position.y);
        sim.set_modes(&[Mode::Rain]);
        assert!(sim.winter_sprites().is_empty());
    }

    #[test]
    fn finished_ball_can_be_picked_up_and_thrown_until_it_breaks() {
        let mut sim = snowy_ground();
        sim.winter.balls.push(super::WinterBall { x: 500.0, y: 50.0, diameter: 30.0, velocity: V2::ZERO, layer_id: 0 });
        sim.winter_step(1.0 / 60.0, V2::new(500.0, 50.0), true);
        assert!(sim.winter.balls.is_empty());
        assert!(sim.winter.working.is_some());
        sim.winter_step(1.0 / 60.0, V2::new(520.0, 100.0), true);
        assert_eq!(sim.winter.working.unwrap().x, 520.0);
        // A controlled fast downward throw must turn the ball into loose snow.
        sim.winter.working.as_mut().unwrap().velocity = V2::new(0.0, -1200.0);
        sim.winter_step(1.0 / 60.0, V2::new(520.0, 100.0), false);
        for _ in 0..20 { sim.winter_step(1.0 / 60.0, V2::new(-1000.0, -1000.0), false); }
        assert!(sim.winter.balls.is_empty());
        assert!(!sim.air.grains.is_empty());
    }

    #[test]
    fn plow_collects_a_ball_and_rolls_it_out_before_turning() {
        let mut sim = snowy_ground();
        let mut outside = false;
        for _ in 0..400 {
            sim.winter_step(0.05, V2::new(-1000.0, -1000.0), false);
            for sprite in sim.winter_sprites().iter().filter(|s| s.ghost[3] == 1.0) {
                outside |= sprite.position.x - sprite.size * 0.46 > sim.width();
            }
            if sim.winter.tractor_direction < 0.0 {
                assert!(outside, "the ball must leave the display before it is released");
                assert_eq!(sim.winter.tractor_snow_area, 0.0);
                assert!(sim.winter.tractor_x > sim.width());
                return;
            }
        }
        panic!("plow failed to complete its pass");
    }

    #[test]
    fn cleaner_waits_offscreen_and_uses_the_changed_interval() {
        let mut sim = snowy_ground();
        sim.winter.initialized = true;
        sim.winter.tractor_active = true;
        sim.winter.tractor_x = sim.width() + 69.0;
        sim.winter.tractor_direction = 1.0;
        sim.set_winter_interval(2);
        let away = V2::new(-1000.0, -1000.0);
        sim.winter_step(0.05, away, false);
        assert!(!sim.winter.tractor_active);
        assert!(sim.winter_sprites().iter().all(|s| s.ghost[0] != -1.0));
        for _ in 0..20 { sim.winter_step(0.05, away, false); }
        assert!(!sim.winter.tractor_active);
        sim.set_winter_interval(1);
        for _ in 0..21 { sim.winter_step(0.05, away, false); }
        assert!(sim.winter.tractor_active);
    }

    #[test]
    fn isolated_snow_peak_has_a_rounded_render_profile() {
        let mut sim = snowy_ground();
        let layer = &mut sim.layers[0];
        for i in 0..layer.count() { layer.set(i, 0.0); }
        let mid = layer.count() / 2;
        layer.set(mid, 100.0);
        let mut profile = vec![0.0; layer.count() * 4];
        layer.write_smoothed(&mut profile, 1.0);
        assert!(profile[mid] < 40.0);
        assert!(profile[mid - 1] > profile[mid - 2] && profile[mid - 2] > profile[mid - 3]);
        assert!(profile[..layer.count()].iter().all(|h| *h >= 0.0));
    }

    #[test]
    fn tall_snowman_has_one_head() {
        let mut sim = snowy_ground();
        for k in 0..5 {
            sim.winter.balls.push(super::WinterBall { x: 400.0, y: 50.0 + k as f32 * 27.6, diameter: 30.0, velocity: V2::ZERO, layer_id: 0 });
        }
        let sprites = sim.winter_ball_sprites();
        let heads: Vec<_> = sprites.iter().filter(|s| s.ghost[0] == -3.0).collect();
        assert_eq!(heads.len(), 1);
        assert_eq!(heads[0].position.y, sim.winter.balls[4].y);
    }

    #[test]
    fn window_impact_breaks_balls_and_can_be_disabled() {
        let mut sim = snowy_ground();
        sim.winter.tractor_wait = 1000.0;
        sim.covers = vec![[450.0, 100.0, 650.0, 400.0]];
        sim.set_winter_window_walls(true);
        let ball = super::WinterBall { x: 420.0, y: 250.0, diameter: 30.0, velocity: V2::new(1000.0, 0.0), layer_id: -1 };
        sim.winter.balls.push(ball);
        let away = V2::new(-1000.0, -1000.0);
        for _ in 0..3 { sim.winter_step(0.05, away, false); }
        assert!(sim.winter.balls.is_empty());
        for _ in 0..24 { sim.winter_step(0.05, away, false); }
        sim.set_winter_window_walls(false);
        sim.winter.balls.push(ball);
        for _ in 0..3 { sim.winter_step(0.05, away, false); }
        assert_eq!(sim.winter.balls.len(), 1);
        assert!(sim.winter.balls[0].x > 450.0);
    }

    #[test]
    fn cleaner_cuts_a_quarter_in_both_directions_even_with_window_and_wind() {
        for direction in [-1.0, 1.0] {
            let mut sim = snowy_ground();
            sim.winter.initialized = true;
            sim.winter.tractor_direction = direction;
            sim.set_winter_wind(1.0);
            sim.covers = vec![[0.0, 0.0, 800.0, 500.0]];
            for _ in 0..90 { sim.winter_step(1.0/60.0, V2::new(-1000.0, -1000.0), false); }
            let x = if direction > 0.0 { 50.0 } else { 750.0 };
            let column = sim.layers[0].clamped_index(x);
            assert!((sim.layers[0].heights[column] - 35.0 * 0.75).abs() < 0.001);
            assert!(sim.winter.tractor_snow_area > 0.0);
            assert!(!sim.winter.plow_track.is_empty());
        }
    }

    #[test]
    fn snowman_stops_to_scoop_real_snow_then_strolls_again() {
        let mut sim = snowy_ground();
        sim.set_winter_cleaner(5);
        let before: f32 = sim.layers[0].heights.iter().sum();
        let away = V2::new(-1000.0, -1000.0);
        for _ in 0..36 { sim.winter_step(0.05, away, false); }
        assert!(sim.winter.snowman_phase >= 0.62 && sim.winter.snowman_phase < 0.74);
        let stopped_x = sim.winter.tractor_x;
        let after_scoop: f32 = sim.layers[0].heights.iter().sum();
        assert!(after_scoop < before, "scoop beat should remove snow from the ground");
        for _ in 0..4 { sim.winter_step(0.05, away, false); }
        assert!((sim.winter.tractor_x - stopped_x).abs() < 0.01, "snowman should stay planted while working");
        for _ in 0..30 { sim.winter_step(0.05, away, false); }
        assert!(sim.winter.snowman_phase < 0.42);
        assert!(sim.winter.tractor_x > stopped_x, "snowman should continue strolling after the shovel cycle");
    }

    #[test]
    fn blower_operator_walks_slower_than_the_plow() {
        let away = V2::new(-1000.0, -1000.0);
        let mut plow = snowy_ground();
        plow.winter.tractor_wait = 1000.0;
        plow.winter.tractor_active = true;
        let plow_start = plow.winter.tractor_x;
        for _ in 0..20 { plow.winter_step(0.05, away, false); }

        let mut operator = snowy_ground();
        operator.set_winter_cleaner(6);
        operator.winter.tractor_wait = 1000.0;
        operator.winter.tractor_active = true;
        let operator_start = operator.winter.tractor_x;
        for _ in 0..20 { operator.winter_step(0.05, away, false); }
        let plow_distance = (plow.winter.tractor_x - plow_start).abs();
        let operator_distance = (operator.winter.tractor_x - operator_start).abs();
        assert!(operator_distance < plow_distance * 0.6);
    }

    #[test]
    fn snowman_keeps_walking_and_never_scoops_empty_ground() {
        let mut sim = super::super::SnowSimulation::new(800.0, 600.0, 0.0);
        sim.set_winter_cleaner(5);
        let away = V2::new(-1000.0, -1000.0);
        let start = sim.winter.tractor_x;
        for _ in 0..30 { sim.winter_step(0.05, away, false); }
        assert!(sim.winter.tractor_x > start + 50.0);
        assert_eq!(sim.winter.tractor_snow_area, 0.0);
        assert!(sim.winter.panel_snow.is_empty());
        assert!(sim.winter.snowman_phase < 0.42);
    }

    #[test]
    fn snowman_throws_collected_snow_into_the_panel_with_bounded_particles() {
        let mut sim = snowy_ground();
        sim.set_winter_cleaner(5);
        sim.set_winter_panel_height(48.0);
        let before: f32 = sim.layers[0].heights.iter().sum::<f32>() * super::super::COLUMN_WIDTH;
        let away = V2::new(-1000.0, -1000.0);
        for _ in 0..52 { sim.winter_step(0.05, away, false); }
        let after: f32 = sim.layers[0].heights.iter().sum::<f32>() * super::super::COLUMN_WIDTH;
        assert!(before > after, "snow should be removed from beneath the shovel");
        assert!(!sim.winter.panel_snow.is_empty());
        assert!(sim.winter.panel_snow.len() <= 60);
        let carried: f32 = sim.winter.panel_snow.iter().map(|flake| flake.area).sum();
        assert!((carried - (before - after)).abs() < 0.1);
        let panel_sprites = sim.winter_panel_sprites();
        assert_eq!(panel_sprites.len(), sim.winter.panel_snow.len());
        assert!(panel_sprites.iter().all(|sprite| sprite.position.y > 0.0));
        for _ in 0..100 { sim.winter_step(0.05, away, false); }
        assert!(sim.winter.panel_snow.is_empty(), "clumps should fall out of the bottom of the taskbar");
    }
}
