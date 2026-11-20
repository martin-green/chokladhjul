//! Game state, animation and rendering.

use std::f32::consts::TAU;

use glow::HasContext;

use crate::audio::Audio;
use crate::gl_util::{Color, EmptyVao, Program, QuadBatch, Texture};
use crate::physics::{
    MAIN_SECTORS, MAX_SPIN_SPEED, MIN_SPIN_SPEED, MainParams, MainWheel, PRIZE_SECTORS, Spinner, SpinnerParams,
    WheelGeometry,
};
use crate::rng::Rng;
use crate::shaders;
use crate::text::SdfFont;
use crate::textures::{self, Image};

pub const VIRTUAL_W: f32 = 1920.0;
pub const VIRTUAL_H: f32 = 1080.0;

const MAIN_CENTER: [f32; 2] = [590.0, 568.0];
const MAIN_RADIUS: f32 = 476.0;
const PRIZE_CENTER: [f32; 2] = [1500.0, 682.0];
const PRIZE_RADIUS: f32 = 288.0;
/// The arrow art is drawn at the same relative size as in the web version.
const ARROW_SIZE: f32 = PRIZE_RADIUS * 2.0 * 350.0 / 640.0;
const NUMBER_POS: [f32; 2] = [1500.0, 156.0];
const NUMBER_SIZE: f32 = 172.0;
const PRIZE_TEXT_POS: [f32; 2] = [1500.0, 298.0];
const PRIZE_TEXT_SIZE: f32 = 92.0;
const LOGO_CENTER: [f32; 2] = [1818.0, 980.0];
const LOGO_SIZE: f32 = 150.0;
/// Fraction of a frame the virtual camera shutter is open (motion blur length).
const SHUTTER: f32 = 0.75;

mod assets {
    pub const WHEEL: &[u8] = include_bytes!("../assets/wof.png");
    pub const PRIZE: &[u8] = include_bytes!("../assets/prizewheel2.png");
    pub const PRIZE_CLASSIC: &[u8] = include_bytes!("../assets/prizewheel.png");
    pub const ARROW: &[u8] = include_bytes!("../assets/arrow.png");
    pub const LOGO: &[u8] = include_bytes!("../assets/tynnered-logo-small.png");
    pub const TICK: &[u8] = include_bytes!("../assets/tick.mp3");
    pub const FANFARE: &[u8] = include_bytes!("../assets/fanfar.mp3");
}

pub fn audio() -> Audio {
    Audio::new(assets::TICK, assets::FANFARE)
}

pub fn window_icon() -> Option<Image> {
    textures::decode_png(assets::LOGO).ok().map(|img| textures::downscale(&img, 64))
}

#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

impl Viewport {
    pub fn scale(&self) -> f32 {
        (self.width as f32 / VIRTUAL_W).min(self.height as f32 / VIRTUAL_H)
    }

    fn offset(&self) -> [f32; 2] {
        let s = self.scale();
        [(self.width as f32 - VIRTUAL_W * s) / 2.0, (self.height as f32 - VIRTUAL_H * s) / 2.0]
    }

    /// Maps virtual pixels to clip space: ndc = pos * xy + zw.
    fn view(&self) -> [f32; 4] {
        let s = self.scale();
        let [ox, oy] = self.offset();
        let (w, h) = (self.width as f32, self.height as f32);
        [2.0 * s / w, -2.0 * s / h, 2.0 * ox / w - 1.0, 1.0 - 2.0 * oy / h]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prize {
    Star,
    First,
    Nothing,
}

impl Prize {
    /// Sector is 0-based clockwise from the top of the prize wheel.
    fn for_sector(sector: usize, classic: bool) -> Prize {
        if classic {
            match sector {
                0 => Prize::Star,
                1 | 7 => Prize::First,
                _ => Prize::Nothing,
            }
        } else if sector.is_multiple_of(2) {
            Prize::Star
        } else {
            Prize::First
        }
    }

    fn label(self) -> Option<&'static str> {
        match self {
            Prize::Star => Some("Stjärnvinst"),
            Prize::First => Some("Förstapris"),
            Prize::Nothing => None,
        }
    }
}

/// Snapshot used by the capture tool.
pub struct DebugState {
    pub main_speed: f64,
    pub flapper: f64,
    pub number_shown: bool,
    pub prize_shown: bool,
    pub prize: Option<Prize>,
}

/// Pop-in / fade-out state of a result label.
#[derive(Default)]
struct Reveal {
    value: Option<usize>,
    visible: bool,
    since: f64,
}

impl Reveal {
    fn show(&mut self, value: usize, now: f64) {
        self.value = Some(value);
        self.visible = true;
        self.since = now;
    }

    fn hide(&mut self, now: f64) {
        if self.visible {
            self.visible = false;
            self.since = now;
        }
    }

    /// (scale, alpha)
    fn anim(&self, now: f64) -> (f32, f32) {
        let t = (now - self.since) as f32;
        if self.value.is_none() {
            (0.0, 0.0)
        } else if self.visible {
            (ease_out_back((t / 0.55).min(1.0), 2.4), (t / 0.12).min(1.0))
        } else {
            let p = (t / 0.3).min(1.0);
            (1.0 - 0.25 * p, 1.0 - p)
        }
    }
}

fn ease_out_back(t: f32, overshoot: f32) -> f32 {
    let u = t - 1.0;
    1.0 + (overshoot + 1.0) * u * u * u + overshoot * u * u
}

fn approach(current: f32, target: f32, rate: f32, dt: f32) -> f32 {
    current + (target - current) * (1.0 - (-rate * dt).exp())
}

struct Confetti {
    pos: [f32; 2],
    vel: [f32; 2],
    rot: f32,
    spin: f32,
    flip: f32,
    flip_speed: f32,
    size: [f32; 2],
    color: [f32; 3],
    sway: f32,
    age: f32,
}

struct Sparkle {
    pos: [f32; 2],
    vel: [f32; 2],
    age: f32,
    life: f32,
    size: f32,
    rot: f32,
    spin: f32,
    color: [f32; 3],
}

const CONFETTI_COLORS: [[f32; 3]; 7] = [
    [1.0, 0.84, 0.2],
    [0.95, 0.22, 0.3],
    [0.2, 0.6, 1.0],
    [0.3, 0.86, 0.4],
    [1.0, 0.45, 0.75],
    [1.0, 1.0, 1.0],
    [1.0, 0.56, 0.1],
];

struct Gfx {
    sprite: Program,
    wheel: Program,
    text: Program,
    background: Program,
    batch: QuadBatch,
    empty: EmptyVao,
    wheel_tex: Texture,
    prize_tex: Texture,
    arrow_tex: Texture,
    logo_tex: Texture,
    flapper_tex: Texture,
    flapper_shadow_tex: Texture,
    flapper_pivot: [f32; 2],
    flapper_size: [f32; 2],
    knob_tex: Texture,
    disc_tex: Texture,
    glow_tex: Texture,
    sparkle_tex: Texture,
    white_tex: Texture,
    font: Option<SdfFont>,
}

fn upload(gl: &glow::Context, img: &Image, mipmaps: bool) -> Result<Texture, String> {
    Texture::rgba(gl, img.width, img.height, &img.pixels, mipmaps)
}

impl Gfx {
    fn new(gl: &glow::Context, geo: &WheelGeometry, classic: bool) -> Result<Self, String> {
        let wheel_art = textures::decode_png(assets::WHEEL)?;
        let wheel = textures::bake_wheel(&wheel_art, 1024);
        let prize = textures::decode_png(if classic { assets::PRIZE_CLASSIC } else { assets::PRIZE })?;
        let arrow = textures::decode_png(assets::ARROW)?;
        let logo = textures::decode_png(assets::LOGO)?;
        let flapper = textures::flapper(geo, 3.0);

        let font = match crate::text::load_system_font() {
            Some((path, data)) => match SdfFont::new(gl, &data, 128.0, 16) {
                Ok(font) => Some(font),
                Err(e) => {
                    crate::warn(&format!("could not load font {path}: {e}"));
                    None
                }
            },
            None => {
                crate::warn("no usable font found, results will not be shown as text");
                None
            }
        };

        Ok(Gfx {
            sprite: Program::new(gl, shaders::QUAD_VS, shaders::SPRITE_FS)?,
            wheel: Program::new(gl, shaders::QUAD_VS, shaders::WHEEL_FS)?,
            text: Program::new(gl, shaders::QUAD_VS, shaders::TEXT_FS)?,
            background: Program::new(gl, shaders::FULLSCREEN_VS, shaders::BACKGROUND_FS)?,
            batch: QuadBatch::new(gl)?,
            empty: EmptyVao::new(gl)?,
            wheel_tex: upload(gl, &wheel, true)?,
            prize_tex: upload(gl, &prize, true)?,
            arrow_tex: upload(gl, &arrow, true)?,
            logo_tex: upload(gl, &logo, true)?,
            flapper_tex: upload(gl, &flapper.body, true)?,
            flapper_shadow_tex: upload(gl, &flapper.shadow, true)?,
            flapper_pivot: flapper.pivot,
            flapper_size: flapper.size,
            knob_tex: upload(gl, &textures::knob(96), true)?,
            disc_tex: upload(gl, &textures::soft_disc(256), true)?,
            glow_tex: upload(gl, &textures::glow(128), true)?,
            sparkle_tex: upload(gl, &textures::sparkle(64), true)?,
            white_tex: upload(gl, &textures::white(), false)?,
            font,
        })
    }
}

struct TextStyle {
    top: [f32; 3],
    bottom: [f32; 3],
    outline: [f32; 3],
    outline_px: f32,
}

/// Parameters for one draw with the wheel shader.
struct WheelDraw<'a> {
    tex: &'a Texture,
    center: [f32; 2],
    size: f32,
    angle: f32,
    blur: f32,
    sectors: f32,
    win: Option<usize>,
    highlight: f32,
    hub: f32,
    gloss: f32,
    shadow: bool,
    lod_bias: f32,
    color: Color,
}

pub struct Game {
    gfx: Gfx,
    audio: Audio,
    rng: Rng,
    main: MainWheel,
    arrow: Spinner,
    classic: bool,
    time: f64,
    runs: u32,
    number: Reveal,
    prize: Reveal,
    spot_main: f32,
    spot_prize: f32,
    burst: f32,
    burst_target: f32,
    burst_color: [f32; 3],
    celebrated: bool,
    confetti: Vec<Confetti>,
    sparkles: Vec<Sparkle>,
    sparkle_accum: f32,
    frame_dt: f32,
}

impl Game {
    pub fn new(gl: &glow::Context, audio: Audio, rng: Rng, classic: bool) -> Result<Self, String> {
        let geo = WheelGeometry::for_radius(MAIN_RADIUS as f64);
        Ok(Game {
            gfx: Gfx::new(gl, &geo, classic)?,
            audio,
            rng,
            main: MainWheel::new(geo, MainParams::default()),
            arrow: Spinner::new(SpinnerParams::default()),
            classic,
            time: 0.0,
            runs: 0,
            number: Reveal::default(),
            prize: Reveal::default(),
            spot_main: 0.0,
            spot_prize: 0.0,
            burst: 0.0,
            burst_target: 0.0,
            burst_color: [1.0, 0.8, 0.3],
            celebrated: false,
            confetti: Vec::new(),
            sparkles: Vec::new(),
            sparkle_accum: 0.0,
            frame_dt: 1.0 / 60.0,
        })
    }

    pub fn spin(&mut self) {
        self.main.spin(self.rng.range(MIN_SPIN_SPEED, MAX_SPIN_SPEED));
        self.arrow.spin(self.rng.range(MIN_SPIN_SPEED, MAX_SPIN_SPEED));
        self.runs += 1;
        self.number.hide(self.time);
        self.prize.hide(self.time);
        self.celebrated = false;
        self.burst_target = 0.0;
    }

    pub fn debug(&self) -> DebugState {
        DebugState {
            main_speed: self.main.omega,
            flapper: self.main.pointer,
            number_shown: self.number.visible,
            prize_shown: self.prize.visible,
            prize: self.current_prize(),
        }
    }

    pub fn update(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, 0.1);
        self.time += dt as f64;
        self.frame_dt = approach(self.frame_dt, dt.clamp(1.0 / 144.0, 1.0 / 30.0), 10.0, dt);

        self.main.step(dt as f64);
        self.arrow.step(dt as f64);

        let loudest = self.main.drain_ticks().map(|t| t.strength).fold(0.0f32, f32::max);
        if loudest > 0.0 {
            self.audio.tick(loudest);
        }

        if self.runs > 0 {
            if !self.number.visible && self.main.is_settled() {
                self.number.show(self.main.result_sector(), self.time);
                let tip = self.main.pointer_tip();
                let at = [MAIN_CENTER[0] + tip.0 as f32, MAIN_CENTER[1] + tip.1 as f32 + 40.0];
                self.sparkle_burst(at, 26, 260.0, [1.0, 0.9, 0.5]);
            }
            if !self.prize.visible && self.arrow.is_settled() {
                self.prize.show(self.arrow.result_sector(), self.time);
                let (s, c) = (self.arrow.angle as f32).sin_cos();
                let reach = ARROW_SIZE * 0.42;
                let at = [PRIZE_CENTER[0] + s * reach, PRIZE_CENTER[1] - c * reach];
                self.sparkle_burst(at, 26, 260.0, [1.0, 0.9, 0.5]);
            }
            if self.number.visible && self.prize.visible && !self.celebrated {
                self.celebrated = true;
                self.celebrate();
            }
        }

        self.spot_main = approach(self.spot_main, if self.number.visible { 1.0 } else { 0.0 }, 5.0, dt);
        self.spot_prize = approach(self.spot_prize, if self.prize.visible { 1.0 } else { 0.0 }, 5.0, dt);
        self.burst = approach(self.burst, self.burst_target, 2.5, dt);

        if self.prize.visible && self.current_prize() == Some(Prize::Star) {
            self.sparkle_accum += dt * 10.0;
            while self.sparkle_accum >= 1.0 {
                self.sparkle_accum -= 1.0;
                let at = [
                    PRIZE_TEXT_POS[0] + self.rng.rangef(-300.0, 300.0),
                    PRIZE_TEXT_POS[1] + self.rng.rangef(-55.0, 45.0),
                ];
                self.sparkles.push(Sparkle {
                    pos: at,
                    vel: [0.0, self.rng.rangef(-30.0, -10.0)],
                    age: 0.0,
                    life: self.rng.rangef(0.5, 1.1),
                    size: self.rng.rangef(18.0, 40.0),
                    rot: 0.0,
                    spin: self.rng.rangef(-1.0, 1.0),
                    color: [1.0, 0.92, 0.6],
                });
            }
        }

        self.update_particles(dt);
    }

    fn current_prize(&self) -> Option<Prize> {
        self.prize.value.map(|s| Prize::for_sector(s, self.classic))
    }

    fn celebrate(&mut self) {
        match self.current_prize() {
            Some(Prize::Star) => {
                self.audio.fanfare();
                self.burst_target = 1.0;
                self.burst_color = [1.0, 0.78, 0.25];
                self.confetti_burst();
                self.sparkle_burst(PRIZE_TEXT_POS, 40, 420.0, [1.0, 0.9, 0.45]);
            }
            Some(Prize::First) => {
                self.burst_target = 0.6;
                self.burst_color = [0.75, 0.9, 1.0];
                self.sparkle_burst(PRIZE_TEXT_POS, 30, 320.0, [0.85, 0.95, 1.0]);
            }
            _ => {}
        }
    }

    fn confetti_burst(&mut self) {
        let cannons = [([1060.0, 1110.0], -58.0f32), ([1940.0, 1110.0], -122.0f32)];
        for (origin, dir_deg) in cannons {
            for _ in 0..170 {
                let a = (dir_deg + self.rng.rangef(-16.0, 16.0)).to_radians();
                let speed = self.rng.rangef(1500.0, 2700.0);
                self.spawn_confetti(origin, [a.cos() * speed, a.sin() * speed]);
            }
        }
        for _ in 0..110 {
            let origin = [self.rng.rangef(1080.0, 1920.0), self.rng.rangef(-320.0, -20.0)];
            let vel = [self.rng.rangef(-60.0, 60.0), self.rng.rangef(0.0, 120.0)];
            self.spawn_confetti(origin, vel);
        }
    }

    fn spawn_confetti(&mut self, pos: [f32; 2], vel: [f32; 2]) {
        let color = CONFETTI_COLORS[(self.rng.next_u64() % CONFETTI_COLORS.len() as u64) as usize];
        let w = self.rng.rangef(9.0, 15.0);
        self.confetti.push(Confetti {
            pos,
            vel,
            rot: self.rng.rangef(0.0, TAU),
            spin: self.rng.rangef(-6.0, 6.0),
            flip: self.rng.rangef(0.0, TAU),
            flip_speed: self.rng.rangef(5.0, 13.0),
            size: [w, w * self.rng.rangef(1.3, 1.9)],
            color,
            sway: self.rng.rangef(0.0, TAU),
            age: 0.0,
        });
    }

    fn sparkle_burst(&mut self, at: [f32; 2], count: usize, speed: f32, color: [f32; 3]) {
        for _ in 0..count {
            let a = self.rng.rangef(0.0, TAU);
            let v = self.rng.rangef(0.25, 1.0) * speed;
            self.sparkles.push(Sparkle {
                pos: at,
                vel: [a.cos() * v, a.sin() * v],
                age: 0.0,
                life: self.rng.rangef(0.5, 1.2),
                size: self.rng.rangef(20.0, 48.0),
                rot: self.rng.rangef(0.0, 1.0),
                spin: self.rng.rangef(-2.0, 2.0),
                color,
            });
        }
    }

    fn update_particles(&mut self, dt: f32) {
        let drag = (-2.4 * dt).exp();
        for c in &mut self.confetti {
            c.age += dt;
            c.vel[1] += 780.0 * dt;
            c.vel[0] *= drag;
            c.vel[1] *= drag;
            c.pos[0] += (c.vel[0] + (c.age * 3.1 + c.sway).sin() * 40.0) * dt;
            c.pos[1] += c.vel[1] * dt;
            c.rot += c.spin * dt;
            c.flip += c.flip_speed * dt;
        }
        self.confetti.retain(|c| c.pos[1] < VIRTUAL_H + 60.0 && c.age < 14.0);

        let drag = (-3.0 * dt).exp();
        for s in &mut self.sparkles {
            s.age += dt;
            s.vel[0] *= drag;
            s.vel[1] *= drag;
            s.pos[0] += s.vel[0] * dt;
            s.pos[1] += s.vel[1] * dt;
            s.rot += s.spin * dt;
        }
        self.sparkles.retain(|s| s.age < s.life);
    }

    pub fn render(&mut self, gl: &glow::Context, vp: Viewport) {
        unsafe {
            gl.viewport(0, 0, vp.width as i32, vp.height as i32);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::BLEND);
            // Premultiplied alpha; colors with zero alpha blend additively.
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
        }
        let view = vp.view();
        let scale = vp.scale();
        let now = self.time as f32;

        self.draw_background(gl, vp);

        // Drop shadows and the logo.
        let g = &mut self.gfx;
        g.sprite.bind(gl);
        g.sprite.set_vec4(gl, "u_view", view);
        g.sprite.set_i32(gl, "u_tex", 0);
        g.disc_tex.bind(gl, 0);
        let main_d = MAIN_RADIUS * 2.0;
        let prize_d = PRIZE_RADIUS * 2.0;
        g.batch.centered([MAIN_CENTER[0] + 14.0, MAIN_CENTER[1] + 22.0], [main_d * 1.07; 2], [0.0, 0.0, 0.0, 0.5]);
        g.batch.centered([PRIZE_CENTER[0] + 12.0, PRIZE_CENTER[1] + 18.0], [prize_d * 1.08; 2], [0.0, 0.0, 0.0, 0.45]);
        g.batch.flush(gl);
        g.logo_tex.bind(gl, 0);
        g.batch.centered(LOGO_CENTER, [LOGO_SIZE; 2], [1.0; 4]);
        g.batch.flush(gl);

        let main_blur = self.main.omega as f32 * self.frame_dt * SHUTTER;
        let arrow_blur = self.arrow.omega as f32 * self.frame_dt * SHUTTER;
        let main_angle = self.main.angle as f32;
        let arrow_angle = self.arrow.angle as f32;
        let number = self.number.value;
        let prize_value = self.prize.value;
        let (spot_main, spot_prize) = (self.spot_main, self.spot_prize);

        let g = &mut self.gfx;
        g.wheel.bind(gl);
        g.wheel.set_vec4(gl, "u_view", view);
        g.wheel.set_i32(gl, "u_tex", 0);
        g.wheel.set_f32(gl, "u_time", now);
        let wheel_tex = g.wheel_tex;
        Self::draw_wheel(gl, g, scale, WheelDraw {
            tex: &wheel_tex,
            center: MAIN_CENTER,
            size: main_d,
            angle: main_angle,
            blur: main_blur,
            sectors: MAIN_SECTORS as f32,
            win: number,
            highlight: spot_main,
            hub: 0.395,
            gloss: 1.0,
            shadow: false,
            lod_bias: 0.0,
            color: [1.0; 4],
        });

        // Flapper with its shadow and pivot knob.
        let geo = self.main.geo;
        let pivot = [MAIN_CENTER[0], MAIN_CENTER[1] - geo.pivot_dist as f32];
        let flap = self.main.pointer as f32;
        g.sprite.bind(gl);
        g.flapper_shadow_tex.bind(gl, 0);
        g.batch.sprite([pivot[0] + 5.0, pivot[1] + 8.0], g.flapper_size, g.flapper_pivot, flap, [0.0, 0.0, 1.0, 1.0], [
            0.0, 0.0, 0.0, 0.5,
        ]);
        g.batch.flush(gl);
        g.flapper_tex.bind(gl, 0);
        g.batch.sprite(pivot, g.flapper_size, g.flapper_pivot, flap, [0.0, 0.0, 1.0, 1.0], [1.0; 4]);
        g.batch.flush(gl);
        g.knob_tex.bind(gl, 0);
        g.batch.centered(pivot, [geo.pointer_half_width as f32 * 1.35; 2], [1.0; 4]);
        g.batch.flush(gl);

        g.wheel.bind(gl);
        let prize_tex = g.prize_tex;
        let arrow_tex = g.arrow_tex;
        Self::draw_wheel(gl, g, scale, WheelDraw {
            tex: &prize_tex,
            center: PRIZE_CENTER,
            size: prize_d,
            angle: 0.0,
            blur: 0.0,
            sectors: PRIZE_SECTORS as f32,
            win: prize_value,
            highlight: spot_prize,
            hub: 0.0,
            gloss: 1.0,
            shadow: false,
            lod_bias: 0.0,
            color: [1.0; 4],
        });
        Self::draw_wheel(gl, g, scale, WheelDraw {
            tex: &arrow_tex,
            center: [PRIZE_CENTER[0] + 9.0, PRIZE_CENTER[1] + 14.0],
            size: ARROW_SIZE,
            angle: arrow_angle,
            blur: arrow_blur,
            sectors: 1.0,
            win: None,
            highlight: 0.0,
            hub: 0.0,
            gloss: 0.0,
            shadow: true,
            lod_bias: 2.5,
            color: [0.0, 0.0, 0.0, 0.5],
        });
        Self::draw_wheel(gl, g, scale, WheelDraw {
            tex: &arrow_tex,
            center: PRIZE_CENTER,
            size: ARROW_SIZE,
            angle: arrow_angle,
            blur: arrow_blur,
            sectors: 1.0,
            win: None,
            highlight: 0.0,
            hub: 0.0,
            gloss: 0.0,
            shadow: false,
            lod_bias: 0.0,
            color: [1.0; 4],
        });
        g.sprite.bind(gl);
        g.knob_tex.bind(gl, 0);
        g.batch.centered(PRIZE_CENTER, [30.0; 2], [1.0; 4]);
        g.batch.flush(gl);

        self.draw_results(gl, vp);
        self.draw_particles(gl);
    }

    fn draw_background(&mut self, gl: &glow::Context, vp: Viewport) {
        let g = &mut self.gfx;
        let b = &g.background;
        b.bind(gl);
        let [ox, oy] = vp.offset();
        b.set_vec2(gl, "u_res", [vp.width as f32, vp.height as f32]);
        b.set_vec3(gl, "u_virtual", [vp.scale(), ox, oy]);
        b.set_f32(gl, "u_time", self.time as f32);
        b.set_vec2(gl, "u_ray_center", MAIN_CENTER);
        let speed = (self.main.omega / MAX_SPIN_SPEED).clamp(0.0, 1.0) as f32;
        b.set_f32(gl, "u_ray_strength", 0.035 + 0.035 * speed);
        b.set_vec2(gl, "u_burst_center", PRIZE_CENTER);
        b.set_f32(gl, "u_burst", self.burst);
        b.set_vec3(gl, "u_burst_color", self.burst_color);
        g.empty.draw_fullscreen(gl);
    }

    fn draw_wheel(gl: &glow::Context, g: &mut Gfx, view_scale: f32, d: WheelDraw) {
        let p = &g.wheel;
        let radius_px = d.size * 0.5 * view_scale;
        let base_lod = (d.tex.width.max(d.tex.height) as f32 / (d.size * view_scale)).log2().max(0.0);
        p.set_f32(gl, "u_angle", d.angle);
        p.set_f32(gl, "u_blur", d.blur);
        p.set_f32(gl, "u_radius_px", radius_px);
        p.set_f32(gl, "u_base_lod", base_lod);
        p.set_f32(gl, "u_lod_bias", d.lod_bias);
        p.set_f32(gl, "u_sectors", d.sectors);
        p.set_f32(gl, "u_win", d.win.map_or(-1.0, |w| w as f32));
        p.set_f32(gl, "u_highlight", d.highlight);
        p.set_f32(gl, "u_hub", d.hub);
        p.set_f32(gl, "u_gloss", d.gloss);
        p.set_f32(gl, "u_shadow", if d.shadow { 1.0 } else { 0.0 });
        d.tex.bind(gl, 0);
        g.batch.centered(d.center, [d.size; 2], d.color);
        g.batch.flush(gl);
    }

    fn draw_results(&mut self, gl: &glow::Context, vp: Viewport) {
        let now = self.time;
        let (n_scale, n_alpha) = self.number.anim(now);
        let (p_scale, p_alpha) = self.prize.anim(now);
        let prize = self.current_prize();
        let pulse = 1.0 + 0.025 * (now as f32 * 4.0).sin();

        // Soft light behind the labels.
        {
            let g = &mut self.gfx;
            g.sprite.bind(gl);
            g.glow_tex.bind(gl, 0);
            if n_alpha > 0.0 {
                let a = 0.22 * n_alpha;
                g.batch.centered(NUMBER_POS, [420.0 * n_scale, 300.0 * n_scale], [a, a, a * 0.9, 0.0]);
            }
            if p_alpha > 0.0 {
                let c = match prize {
                    Some(Prize::Star) => [1.0, 0.75, 0.2],
                    Some(Prize::First) => [0.6, 0.8, 1.0],
                    _ => [0.0; 3],
                };
                let a = 0.4 * p_alpha;
                g.batch.centered(PRIZE_TEXT_POS, [820.0 * p_scale, 240.0 * p_scale], [
                    c[0] * a,
                    c[1] * a,
                    c[2] * a,
                    0.0,
                ]);
            }
            g.batch.flush(gl);
        }

        if self.gfx.font.is_none() {
            return;
        }
        if let Some(sector) = self.number.value
            && n_alpha > 0.0
        {
            let style = TextStyle {
                top: [1.0, 1.0, 1.0],
                bottom: [0.78, 0.9, 1.0],
                outline: [0.05, 0.12, 0.2],
                outline_px: 7.0,
            };
            let text = (sector + 1).to_string();
            self.draw_text(gl, vp, &text, NUMBER_POS, NUMBER_SIZE, n_scale, n_alpha, &style);
        }
        if let Some(label) = prize.and_then(Prize::label)
            && p_alpha > 0.0
        {
            let style = if prize == Some(Prize::Star) {
                TextStyle {
                    top: [1.0, 0.97, 0.62],
                    bottom: [1.0, 0.62, 0.05],
                    outline: [0.3, 0.13, 0.0],
                    outline_px: 6.0,
                }
            } else {
                TextStyle {
                    top: [1.0, 1.0, 1.0],
                    bottom: [0.72, 0.84, 0.98],
                    outline: [0.06, 0.12, 0.3],
                    outline_px: 6.0,
                }
            };
            let wobble = if prize == Some(Prize::Star) && self.prize.visible { pulse } else { 1.0 };
            self.draw_text(gl, vp, label, PRIZE_TEXT_POS, PRIZE_TEXT_SIZE, p_scale * wobble, p_alpha, &style);
        }
    }

    /// Draws `text` centred on `center` (cap height centred vertically).
    #[allow(clippy::too_many_arguments)]
    fn draw_text(
        &mut self,
        gl: &glow::Context,
        vp: Viewport,
        text: &str,
        center: [f32; 2],
        size: f32,
        scale: f32,
        alpha: f32,
        style: &TextStyle,
    ) {
        let g = &mut self.gfx;
        let Some(font) = &g.font else { return };
        if scale <= 0.01 {
            return;
        }
        let (glyphs, width) = font.layout(text, size);
        let cap = font.cap_height(size);
        let origin = [-width / 2.0, cap / 2.0];
        let px_per_unit = 2.0 * font.spread * (size * scale / font.raster_px) * vp.scale();
        let p = &g.text;
        p.bind(gl);
        p.set_vec4(gl, "u_view", vp.view());
        p.set_i32(gl, "u_tex", 0);
        p.set_f32(gl, "u_px_per_unit", px_per_unit);
        font.texture.bind(gl, 0);

        let premul = |c: [f32; 3], a: f32| [c[0] * a, c[1] * a, c[2] * a, a];
        let emit = |batch: &mut QuadBatch, offset: [f32; 2], colors: &dyn Fn(f32) -> Color| {
            for gq in &glyphs {
                let corners = [
                    [gq.pos[0], gq.pos[1]],
                    [gq.pos[0] + gq.size[0], gq.pos[1]],
                    [gq.pos[0] + gq.size[0], gq.pos[1] + gq.size[1]],
                    [gq.pos[0], gq.pos[1] + gq.size[1]],
                ];
                let pos = corners.map(|c| {
                    [
                        center[0] + offset[0] + (origin[0] + c[0]) * scale,
                        center[1] + offset[1] + (origin[1] + c[1]) * scale,
                    ]
                });
                let col = corners.map(|c| colors(((origin[1] + c[1]) + cap / 2.0) / cap));
                let uv = [
                    [gq.uv[0], gq.uv[1]],
                    [gq.uv[2], gq.uv[1]],
                    [gq.uv[2], gq.uv[3]],
                    [gq.uv[0], gq.uv[3]],
                ];
                batch.quad(pos, uv, col);
            }
        };

        // Soft drop shadow.
        let shadow = premul([0.0; 3], 0.55 * alpha);
        p.set_f32(gl, "u_outline", style.outline_px * scale);
        p.set_vec4(gl, "u_outline_color", shadow);
        p.set_f32(gl, "u_soft", 6.0 * vp.scale());
        emit(&mut g.batch, [5.0 * scale, 8.0 * scale], &|_| shadow);
        g.batch.flush(gl);

        // Outlined gradient fill.
        p.set_vec4(gl, "u_outline_color", premul(style.outline, alpha));
        p.set_f32(gl, "u_soft", 0.0);
        let (top, bottom) = (style.top, style.bottom);
        emit(&mut g.batch, [0.0, 0.0], &|t| {
            let t = t.clamp(0.0, 1.0);
            premul(std::array::from_fn(|k| top[k] + (bottom[k] - top[k]) * t), alpha)
        });
        g.batch.flush(gl);
    }

    fn draw_particles(&mut self, gl: &glow::Context) {
        let g = &mut self.gfx;
        if !self.confetti.is_empty() {
            g.sprite.bind(gl);
            g.white_tex.bind(gl, 0);
            for c in &self.confetti {
                let flip = c.flip.cos();
                let shade = 0.55 + 0.45 * flip.abs();
                let fade = (1.0 - (c.age - 11.0) / 3.0).clamp(0.0, 1.0);
                let col = [c.color[0] * shade * fade, c.color[1] * shade * fade, c.color[2] * shade * fade, fade];
                let size = [c.size[0] * flip.abs().max(0.08), c.size[1]];
                g.batch.sprite(c.pos, size, [0.5, 0.5], c.rot, [0.0, 0.0, 1.0, 1.0], col);
            }
            g.batch.flush(gl);
        }
        if !self.sparkles.is_empty() {
            g.sprite.bind(gl);
            g.sparkle_tex.bind(gl, 0);
            for s in &self.sparkles {
                let t = s.age / s.life;
                let a = (std::f32::consts::PI * t).sin();
                let size = s.size * (0.6 + 0.4 * a);
                let col = [s.color[0] * a, s.color[1] * a, s.color[2] * a, 0.0];
                g.batch.sprite(s.pos, [size; 2], [0.5, 0.5], s.rot, [0.0, 0.0, 1.0, 1.0], col);
            }
            g.batch.flush(gl);
        }
    }
}
