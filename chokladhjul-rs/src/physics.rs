//! Wheel physics.
//!
//! The main wheel is a rigid disc with pegs on its rim that collide with a
//! spring-loaded flapper (the pointer). Contacts are penalty springs and the
//! whole thing is integrated with small fixed substeps, which makes the end of
//! a spin behave like a real wheel: the pegs brake it tick by tick and a peg
//! that cannot quite pass the flapper is pushed back.
//!
//! Coordinates are virtual pixels relative to the wheel centre with y pointing
//! down. All angles are clockwise-positive (the same convention as a 2D canvas
//! rotation), measured from straight up.

use std::f64::consts::TAU;

pub const MAIN_SECTORS: usize = 24;
pub const PRIZE_SECTORS: usize = 8;

type V2 = (f64, f64);

fn sub(a: V2, b: V2) -> V2 {
    (a.0 - b.0, a.1 - b.1)
}

fn dot(a: V2, b: V2) -> f64 {
    a.0 * b.0 + a.1 * b.1
}

/// 2D cross product; positive means a clockwise torque in y-down space.
fn cross(r: V2, f: V2) -> f64 {
    r.0 * f.1 - r.1 * f.0
}

/// Velocity of a point at `r` rotating clockwise with angular speed `w`.
fn spin_velocity(w: f64, r: V2) -> V2 {
    (-w * r.1, w * r.0)
}

fn rotate(p: V2, sin: f64, cos: f64) -> V2 {
    (p.0 * cos - p.1 * sin, p.0 * sin + p.1 * cos)
}

fn closest_on_segment(a: V2, b: V2, p: V2) -> V2 {
    let ab = sub(b, a);
    let t = (dot(sub(p, a), ab) / dot(ab, ab)).clamp(0.0, 1.0);
    (a.0 + ab.0 * t, a.1 + ab.1 * t)
}

/// Coulomb + viscous friction that can stop, but never reverse, the motion.
fn apply_friction(omega: f64, accel: f64, friction_const: f64, friction_visc: f64, h: f64) -> f64 {
    let free = omega + accel * h;
    let loss = (friction_const + friction_visc * omega.abs()) * h;
    if free.abs() <= loss { 0.0 } else { free - loss * free.signum() }
}

/// Sector (0-based, clockwise from the top) that contains `local_angle`.
fn sector_at(local_angle: f64, sectors: usize) -> usize {
    let step = TAU / sectors as f64;
    ((local_angle / step).round() as i64).rem_euclid(sectors as i64) as usize
}

#[derive(Clone, Copy, Debug)]
pub struct WheelGeometry {
    /// Distance from the centre to the peg centres.
    pub peg_ring: f64,
    pub peg_radius: f64,
    /// Distance from the wheel centre up to the flapper pivot.
    pub pivot_dist: f64,
    /// Pivot to tip.
    pub pointer_len: f64,
    /// Half width of the flapper at the pivot; it tapers to a point at the tip.
    pub pointer_half_width: f64,
    pub pointer_edge_radius: f64,
}

impl WheelGeometry {
    pub fn for_radius(radius: f64) -> Self {
        // Proportions match the baked rim texture (see textures.rs).
        let peg_ring = radius * crate::textures::PEG_RING_FRAC as f64;
        let peg_radius = radius * crate::textures::PEG_RADIUS_FRAC as f64;
        let pivot_dist = radius + radius * 0.096;
        let tip = peg_ring - peg_radius * 0.42;
        WheelGeometry {
            peg_ring,
            peg_radius,
            pivot_dist,
            pointer_len: pivot_dist - tip,
            pointer_half_width: radius * 0.042,
            pointer_edge_radius: 1.0,
        }
    }

    pub fn pivot(&self) -> V2 {
        (0.0, -self.pivot_dist)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MainParams {
    pub wheel_inertia: f64,
    /// Constant bearing friction, rad/s².
    pub friction_const: f64,
    /// Air drag style friction, 1/s.
    pub friction_visc: f64,
    pub pointer_inertia: f64,
    pub pointer_stiffness: f64,
    pub pointer_damping: f64,
    /// Rubber stops limiting the flapper swing.
    pub pointer_stop: f64,
    pub stop_stiffness: f64,
    pub stop_damping: f64,
    pub contact_stiffness: f64,
    pub contact_damping: f64,
    pub substep: f64,
    /// Time from rest to full speed when spun.
    pub spin_up_time: f64,
}

impl Default for MainParams {
    fn default() -> Self {
        let pointer_inertia = 1.0;
        let pointer_freq = 2.0 * std::f64::consts::PI * 7.5;
        let pointer_stiffness = pointer_inertia * pointer_freq * pointer_freq;
        MainParams {
            wheel_inertia: 15000.0,
            friction_const: 0.15,
            friction_visc: 0.08,
            pointer_inertia,
            pointer_stiffness,
            pointer_damping: 2.0 * 0.16 * (pointer_stiffness * pointer_inertia).sqrt(),
            pointer_stop: 0.75,
            stop_stiffness: 200.0 * pointer_stiffness,
            stop_damping: 1.4 * (200.0 * pointer_stiffness * pointer_inertia).sqrt(),
            contact_stiffness: 1500.0,
            contact_damping: 0.7,
            substep: 1.0 / 10_000.0,
            spin_up_time: 0.45,
        }
    }
}

/// A peg hitting the flapper, for sound and visual feedback.
#[derive(Clone, Copy, Debug)]
pub struct Tick {
    /// 0..1, based on impact speed.
    pub strength: f32,
}

#[derive(Clone, Debug)]
pub struct MainWheel {
    pub geo: WheelGeometry,
    pub params: MainParams,
    /// Wheel rotation, wrapped to [0, TAU).
    pub angle: f64,
    pub omega: f64,
    /// Flapper rotation about its pivot.
    pub pointer: f64,
    pub pointer_vel: f64,
    drive_target: f64,
    drive_accel: f64,
    stuck_time: f64,
    contact_mask: u32,
    last_contact: [f64; MAIN_SECTORS],
    time: f64,
    ticks: Vec<Tick>,
    pub max_penetration: f64,
}

impl MainWheel {
    pub fn new(geo: WheelGeometry, params: MainParams) -> Self {
        MainWheel {
            geo,
            params,
            angle: 0.0,
            omega: 0.0,
            pointer: 0.0,
            pointer_vel: 0.0,
            drive_target: 0.0,
            drive_accel: 0.0,
            stuck_time: 1.0,
            contact_mask: 0,
            last_contact: [-1.0; MAIN_SECTORS],
            time: 0.0,
            ticks: Vec::new(),
            max_penetration: 0.0,
        }
    }

    /// Spin up to `target` rad/s. Like the original, a weaker push than the
    /// current speed does nothing.
    pub fn spin(&mut self, target: f64) {
        if target > self.omega {
            self.drive_target = target;
            self.drive_accel = target / self.params.spin_up_time;
            self.stuck_time = 0.0;
        }
    }

    pub fn is_driving(&self) -> bool {
        self.drive_target > 0.0
    }

    pub fn is_settled(&self) -> bool {
        !self.is_driving() && self.omega == 0.0 && self.stuck_time > 0.35 && self.pointer_vel.abs() < 0.4
    }

    pub fn drain_ticks(&mut self) -> std::vec::Drain<'_, Tick> {
        self.ticks.drain(..)
    }

    /// Flapper tip in wheel-centre coordinates.
    pub fn pointer_tip(&self) -> V2 {
        let (s, c) = self.pointer.sin_cos();
        let t = rotate((0.0, self.geo.pointer_len), s, c);
        let p = self.geo.pivot();
        (p.0 + t.0, p.1 + t.1)
    }

    /// Winning sector, 0-based clockwise from sector "1". Uses the actual tip
    /// position, so a flapper resting against a peg reads the right sector.
    pub fn result_sector(&self) -> usize {
        let tip = self.pointer_tip();
        let tip_angle = tip.0.atan2(-tip.1);
        sector_at(tip_angle - self.angle, MAIN_SECTORS)
    }

    pub fn step(&mut self, dt: f64) {
        if dt <= 0.0 {
            return;
        }
        let n = (dt / self.params.substep).ceil().max(1.0) as usize;
        let h = dt / n as f64;
        for _ in 0..n {
            self.substep(h);
        }
    }

    fn substep(&mut self, h: f64) {
        let g = self.geo;
        let p = self.params;
        self.time += h;

        let pivot = g.pivot();
        let (s, c) = self.pointer.sin_cos();
        let to_world = |v: V2| {
            let r = rotate(v, s, c);
            (pivot.0 + r.0, pivot.1 + r.1)
        };
        let tip = to_world((0.0, g.pointer_len));
        let edges = [
            (to_world((-g.pointer_half_width, 0.0)), tip),
            (to_world((g.pointer_half_width, 0.0)), tip),
        ];

        let step = TAU / MAIN_SECTORS as f64;
        // Pegs sit between sectors, at local angles (i + 0.5) * step.
        let nearest = ((-self.angle) / step - 0.5).round() as i64;
        let reach = g.peg_radius + g.pointer_edge_radius;
        let mut torque_wheel = 0.0;
        let mut torque_pointer = 0.0;
        let mut mask = 0u32;

        for di in -1..=1 {
            let i = (nearest + di).rem_euclid(MAIN_SECTORS as i64) as usize;
            let phi = (i as f64 + 0.5) * step + self.angle;
            let (sp, cp) = phi.sin_cos();
            let peg = (g.peg_ring * sp, -g.peg_ring * cp);
            let v_peg = spin_velocity(self.omega, peg);
            for &(a, b) in &edges {
                let q = closest_on_segment(a, b, peg);
                let d = sub(peg, q);
                let dist = dot(d, d).sqrt();
                let pen = reach - dist;
                if pen <= 0.0 || dist < 1e-9 {
                    continue;
                }
                self.max_penetration = self.max_penetration.max(pen);
                let n = (d.0 / dist, d.1 / dist);
                let rq = sub(q, pivot);
                let v_q = spin_velocity(self.pointer_vel, rq);
                let vn = dot(sub(v_peg, v_q), n);
                let f = (p.contact_stiffness * pen - p.contact_damping * vn).max(0.0);
                let force = (n.0 * f, n.1 * f);
                torque_wheel += cross(peg, force);
                torque_pointer -= cross(rq, force);

                let bit = 1u32 << i;
                if self.contact_mask & bit == 0 && mask & bit == 0 {
                    let since = self.time - self.last_contact[i];
                    if since > 0.12 && vn < -15.0 {
                        let strength = ((-vn) / 700.0).clamp(0.12, 1.0) as f32;
                        self.ticks.push(Tick { strength });
                    }
                }
                mask |= bit;
                self.last_contact[i] = self.time;
            }
        }
        self.contact_mask = mask;

        // Wheel.
        let mut accel = torque_wheel / p.wheel_inertia;
        if self.drive_target > 0.0 {
            accel += self.drive_accel;
        }
        self.omega = apply_friction(self.omega, accel, p.friction_const, p.friction_visc, h);
        if self.drive_target > 0.0 && self.omega >= self.drive_target {
            self.drive_target = 0.0;
        }
        self.angle = (self.angle + self.omega * h).rem_euclid(TAU);
        if self.omega == 0.0 {
            self.stuck_time += h;
        } else {
            self.stuck_time = 0.0;
        }

        // Flapper: damped spring about its pivot, limited by rubber stops.
        let mut torque = torque_pointer - p.pointer_stiffness * self.pointer - p.pointer_damping * self.pointer_vel;
        let over = self.pointer.abs() - p.pointer_stop;
        if over > 0.0 {
            torque -= self.pointer.signum() * p.stop_stiffness * over + p.stop_damping * self.pointer_vel;
        }
        let pointer_accel = torque / p.pointer_inertia;
        self.pointer_vel += pointer_accel * h;
        self.pointer += self.pointer_vel * h;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SpinnerParams {
    pub friction_const: f64,
    pub friction_visc: f64,
    pub spin_up_time: f64,
}

impl Default for SpinnerParams {
    fn default() -> Self {
        SpinnerParams { friction_const: 0.5, friction_visc: 0.12, spin_up_time: 0.35 }
    }
}

/// Free spinning arrow on the prize wheel.
#[derive(Clone, Debug)]
pub struct Spinner {
    pub params: SpinnerParams,
    pub angle: f64,
    pub omega: f64,
    drive_target: f64,
    drive_accel: f64,
    stopped_time: f64,
}

impl Spinner {
    pub fn new(params: SpinnerParams) -> Self {
        Spinner { params, angle: 0.0, omega: 0.0, drive_target: 0.0, drive_accel: 0.0, stopped_time: 1.0 }
    }

    pub fn spin(&mut self, target: f64) {
        if target > self.omega {
            self.drive_target = target;
            self.drive_accel = target / self.params.spin_up_time;
            self.stopped_time = 0.0;
        }
    }

    pub fn is_settled(&self) -> bool {
        self.drive_target == 0.0 && self.omega == 0.0 && self.stopped_time > 0.2
    }

    pub fn result_sector(&self) -> usize {
        sector_at(self.angle, PRIZE_SECTORS)
    }

    pub fn step(&mut self, dt: f64) {
        let n = (dt / 0.001).ceil().max(1.0) as usize;
        let h = dt / n as f64;
        for _ in 0..n {
            let accel = if self.drive_target > 0.0 { self.drive_accel } else { 0.0 };
            self.omega =
                apply_friction(self.omega, accel, self.params.friction_const, self.params.friction_visc, h);
            if self.drive_target > 0.0 && self.omega >= self.drive_target {
                self.drive_target = 0.0;
            }
            self.angle = (self.angle + self.omega * h).rem_euclid(TAU);
            if self.omega == 0.0 {
                self.stopped_time += h;
            } else {
                self.stopped_time = 0.0;
            }
        }
    }
}

/// Spin speeds (rad/s) used by the game; same range as the original web
/// version (5.6..13.6 degrees per frame at 60 fps).
pub const MIN_SPIN_SPEED: f64 = 5.9;
pub const MAX_SPIN_SPEED: f64 = 14.2;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    const R: f64 = 480.0;

    struct SpinStats {
        duration: f64,
        revolutions: f64,
        ticks: usize,
        max_pointer: f64,
        last_tick_gaps: Vec<f64>,
        sector: usize,
    }

    fn run_spin(wheel: &mut MainWheel, speed: f64) -> SpinStats {
        let dt = 1.0 / 60.0;
        let mut t = 0.0;
        let mut travelled = 0.0;
        let mut ticks = 0;
        let mut tick_times = Vec::new();
        let mut max_pointer: f64 = 0.0;
        wheel.spin(speed);
        while t < 40.0 {
            wheel.step(dt);
            t += dt;
            travelled += wheel.omega * dt;
            for _ in wheel.drain_ticks() {
                ticks += 1;
                tick_times.push(t);
            }
            max_pointer = max_pointer.max(wheel.pointer.abs());
            assert!(wheel.angle.is_finite() && wheel.omega.is_finite());
            assert!(wheel.pointer.is_finite() && wheel.pointer_vel.is_finite());
            if wheel.is_settled() {
                break;
            }
        }
        let gaps = tick_times.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>();
        let last = gaps.iter().rev().take(6).rev().copied().collect();
        SpinStats {
            duration: t,
            revolutions: travelled / TAU,
            ticks,
            max_pointer,
            last_tick_gaps: last,
            sector: wheel.result_sector(),
        }
    }

    #[test]
    fn sector_formula_matches_original_game() {
        // Original: (n + floor(0.5 - angle_deg * n / 360)) % n + 1 with the pointer at the top.
        let mut wheel = MainWheel::new(WheelGeometry::for_radius(R), MainParams::default());
        for k in 0..720 {
            let deg = k as f64 * 0.5 + 0.1;
            wheel.angle = deg.to_radians();
            let original = (24.0 + (0.5 - deg * 24.0 / 360.0).floor()).rem_euclid(24.0) as usize + 1;
            assert_eq!(wheel.result_sector() + 1, original, "angle {deg}");
        }
    }

    #[test]
    fn pointer_clears_pegs_at_rest() {
        let wheel = MainWheel::new(WheelGeometry::for_radius(R), MainParams::default());
        let mut w = wheel.clone();
        w.step(0.5);
        assert!(w.pointer.abs() < 1e-6, "pointer must hang free between pegs at angle 0");
    }

    #[test]
    fn slow_peg_passage_clears_inside_stops() {
        // Push the wheel gently past a few pegs and record how far the flapper must swing.
        let mut wheel = MainWheel::new(WheelGeometry::for_radius(R), MainParams::default());
        wheel.params.friction_const = 0.0;
        wheel.params.friction_visc = 0.0;
        let mut max_flap: f64 = 0.0;
        let start = wheel.angle;
        let mut t = 0.0;
        while t < 3.0 {
            wheel.omega = 0.4;
            wheel.step(1.0 / 240.0);
            t += 1.0 / 240.0;
            max_flap = max_flap.max(wheel.pointer.abs());
        }
        println!("clearance deflection {:.1} deg over {:.1} deg of travel", max_flap.to_degrees(), (wheel.angle - start).to_degrees());
        assert!(max_flap < wheel.params.pointer_stop * 0.95, "pegs need the flapper to pass its stops");
    }

    #[test]
    fn spins_are_stable_and_realistic() {
        let mut rng = Rng::new(7);
        let mut wheel = MainWheel::new(WheelGeometry::for_radius(R), MainParams::default());
        let mut durations = Vec::new();
        let mut revs = Vec::new();
        for i in 0..60 {
            let speed = rng.range(MIN_SPIN_SPEED, MAX_SPIN_SPEED);
            let s = run_spin(&mut wheel, speed);
            assert!(s.duration < 30.0, "spin {i} never settled");
            assert!(s.max_pointer < wheel.params.pointer_stop + 0.12, "flapper over-deflected: {}", s.max_pointer);
            assert!(wheel.max_penetration < 4.0, "deep peg penetration: {}", wheel.max_penetration);
            assert!(s.ticks as f64 >= s.revolutions * 20.0, "missing ticks");
            if i < 6 {
                println!(
                    "speed {speed:5.2} -> {:5.2}s, {:4.1} revs, {} ticks, max flap {:4.1} deg, sector {}, last gaps {:?}",
                    s.duration,
                    s.revolutions,
                    s.ticks,
                    s.max_pointer.to_degrees(),
                    s.sector + 1,
                    s.last_tick_gaps.iter().map(|g| (g * 1000.0).round() / 1000.0).collect::<Vec<_>>()
                );
            }
            durations.push(s.duration);
            revs.push(s.revolutions);
        }
        let min = durations.iter().copied().fold(f64::MAX, f64::min);
        let max = durations.iter().copied().fold(0.0, f64::max);
        let rmin = revs.iter().copied().fold(f64::MAX, f64::min);
        let rmax = revs.iter().copied().fold(0.0, f64::max);
        println!("durations {min:.2}..{max:.2}s, revolutions {rmin:.1}..{rmax:.1}, max pen {:.2}", wheel.max_penetration);
        assert!(min > 4.0 && max < 20.0);
        assert!(rmin > 2.0);
    }

    #[test]
    fn outcomes_are_uniform() {
        let mut rng = Rng::new(42);
        let mut wheel = MainWheel::new(WheelGeometry::for_radius(R), MainParams::default());
        wheel.params.substep = 1.0 / 5000.0;
        let spins = 960;
        let mut counts = [0usize; MAIN_SECTORS];
        for _ in 0..spins {
            let s = run_spin(&mut wheel, rng.range(MIN_SPIN_SPEED, MAX_SPIN_SPEED));
            counts[s.sector] += 1;
        }
        let expected = spins as f64 / MAIN_SECTORS as f64;
        let chi2: f64 = counts.iter().map(|&c| (c as f64 - expected).powi(2) / expected).sum();
        println!("counts {counts:?} chi2 {chi2:.1}");
        // 23 degrees of freedom, p = 0.001 critical value is 49.7.
        assert!(chi2 < 49.7, "biased outcome distribution: chi2 = {chi2}");
    }

    #[test]
    fn arrow_spins_and_stops() {
        let mut rng = Rng::new(3);
        let mut arrow = Spinner::new(SpinnerParams::default());
        let mut counts = [0usize; PRIZE_SECTORS];
        for _ in 0..400 {
            arrow.spin(rng.range(MIN_SPIN_SPEED, MAX_SPIN_SPEED));
            let mut t = 0.0;
            while !arrow.is_settled() {
                arrow.step(1.0 / 60.0);
                t += 1.0 / 60.0;
                assert!(t < 30.0);
            }
            assert!(t > 4.0, "arrow stopped too quickly: {t}");
            counts[arrow.result_sector()] += 1;
        }
        assert!(counts.iter().all(|&c| c > 25), "{counts:?}");
    }
}
