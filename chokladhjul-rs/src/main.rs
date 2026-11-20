#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod game;
mod gl_util;
mod physics;
mod rng;
mod shaders;
mod text;
mod textures;

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use glow::HasContext;
use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{
    ContextApi, ContextAttributesBuilder, GlProfile, NotCurrentGlContext, PossiblyCurrentContext, Version,
};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use raw_window_handle::HasWindowHandle;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Icon, Window, WindowAttributes, WindowId};

use game::{Game, Viewport};

// Ask hybrid-graphics laptops (NVIDIA Optimus / AMD) to run us on the discrete GPU.
// build.rs exports these symbols from the executable.
#[cfg(all(windows, target_env = "msvc"))]
#[allow(non_upper_case_globals)]
#[unsafe(no_mangle)]
#[used]
pub static NvOptimusEnablement: u32 = 1;

#[cfg(all(windows, target_env = "msvc"))]
#[allow(non_upper_case_globals)]
#[unsafe(no_mangle)]
#[used]
pub static AmdPowerXpressRequestHighPerformance: u32 = 1;

pub fn warn(msg: &str) {
    eprintln!("chokladhjul: {msg}");
}

fn fatal(msg: &str) -> ! {
    warn(msg);
    let log = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("chokladhjul-error.txt")))
        .unwrap_or_else(|| std::env::temp_dir().join("chokladhjul-error.txt"));
    let _ = std::fs::write(&log, format!("{msg}\n"));
    std::process::exit(1);
}

#[derive(Clone, Debug, Default)]
struct Options {
    windowed: bool,
    classic: bool,
    sound: bool,
    seed: Option<u64>,
    capture: Option<PathBuf>,
    bench: bool,
}

const USAGE: &str = concat!(
    "Chokladhjul ",
    env!("APP_VERSION"),
    " - tryck på mellanslag eller musknappen för att snurra.

Användning: chokladhjul [flaggor]
  --windowed      starta i fönster istället för helskärm
  --classic       klassiskt vinsthjul (1 stjärnvinst, 2 förstapris, resten nitlotter)
  --no-sound      stäng av ljudet
  --seed <n>      fast slumpfrö
  --capture <dir> spara skärmdumpar av en snurr som PNG och avsluta
  --bench         mät renderingstiden och avsluta

Tangenter: Mellanslag/musknapp = snurra, F11/F = helskärm, Esc = lämna helskärm/avsluta"
);

impl Options {
    fn parse() -> Self {
        let mut o = Options { sound: true, ..Default::default() };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--windowed" => o.windowed = true,
                "--classic" => o.classic = true,
                "--no-sound" => o.sound = false,
                "--bench" => o.bench = true,
                "--seed" => o.seed = args.next().and_then(|s| s.parse().ok()),
                "--capture" => o.capture = args.next().map(PathBuf::from),
                "-h" | "--help" => {
                    println!("{USAGE}");
                    std::process::exit(0);
                }
                other => fatal(&format!("okänd flagga {other}\n\n{USAGE}")),
            }
        }
        o
    }
}

struct Gpu {
    // Field order matters: the GL objects must go before the context and window.
    gl: glow::Context,
    surface: Surface<WindowSurface>,
    context: PossiblyCurrentContext,
    window: Window,
}

fn create_gpu(event_loop: &ActiveEventLoop, attrs: WindowAttributes) -> Result<Gpu, String> {
    let template = ConfigTemplateBuilder::new().with_alpha_size(8);
    let (window, config) = DisplayBuilder::new()
        .with_window_attributes(Some(attrs))
        .build(event_loop, template, |configs| {
            configs
                .min_by_key(|c| (!c.hardware_accelerated() as u8, c.num_samples()))
                .expect("no OpenGL configs")
        })
        .map_err(|e| format!("could not create window: {e}"))?;
    let window = window.ok_or("could not create window")?;
    let raw = window.window_handle().map_err(|e| e.to_string())?.as_raw();
    let display = config.display();

    let attempts = [
        ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
            .with_profile(GlProfile::Core)
            .build(Some(raw)),
        ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
            .build(Some(raw)),
    ];
    let not_current = attempts
        .iter()
        .find_map(|a| unsafe { display.create_context(&config, a).ok() })
        .ok_or("OpenGL 3.3 is not available on this computer")?;

    let surface_attrs = window
        .build_surface_attributes(SurfaceAttributesBuilder::<WindowSurface>::new())
        .map_err(|e| e.to_string())?;
    let surface = unsafe { display.create_window_surface(&config, &surface_attrs) }.map_err(|e| e.to_string())?;
    let context = not_current.make_current(&surface).map_err(|e| e.to_string())?;
    let gl = unsafe { glow::Context::from_loader_function_cstr(|s| display.get_proc_address(s)) };
    Ok(Gpu { gl, surface, context, window })
}

struct Running {
    game: Game,
    gpu: Gpu,
    last_frame: Instant,
    vsync: bool,
    fast_frames: u32,
    limiter: bool,
}

impl Running {
    fn viewport(&self) -> Option<Viewport> {
        let size = self.gpu.window.inner_size();
        (size.width > 0 && size.height > 0).then_some(Viewport { width: size.width, height: size.height })
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.game.update(dt);

        let Some(vp) = self.viewport() else {
            std::thread::sleep(Duration::from_millis(16));
            return;
        };
        self.game.render(&self.gpu.gl, vp);
        if let Err(e) = self.gpu.surface.swap_buffers(&self.gpu.context) {
            warn(&format!("swap failed: {e}"));
        }

        // Fall back to a sleep based 60 fps limiter if vsync is not honoured.
        if !self.limiter {
            self.fast_frames = if dt < 1.0 / 90.0 { self.fast_frames + 1 } else { 0 };
            if !self.vsync || self.fast_frames > 30 {
                self.limiter = true;
            }
        }
        if self.limiter {
            let target = self.last_frame + Duration::from_secs_f64(1.0 / 60.0);
            if let Some(wait) = target.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
        }
    }

    fn toggle_fullscreen(&self) {
        let w = &self.gpu.window;
        if w.fullscreen().is_some() {
            w.set_fullscreen(None);
            w.set_cursor_visible(true);
        } else {
            w.set_fullscreen(Some(Fullscreen::Borderless(None)));
            w.set_cursor_visible(false);
        }
    }
}

struct App {
    opts: Options,
    running: Option<Running>,
}

impl App {
    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let offscreen = self.opts.capture.is_some();
        let mut attrs = Window::default_attributes()
            .with_title(concat!("Chokladhjul ", env!("APP_VERSION")))
            .with_inner_size(LogicalSize::new(1600.0, 900.0))
            .with_visible(!offscreen);
        if !self.opts.windowed && !offscreen && !self.opts.bench {
            attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        if let Some(icon) = game::window_icon() {
            attrs = attrs.with_window_icon(Icon::from_rgba(icon.unpremultiplied(), icon.width, icon.height).ok());
        }
        let gpu = create_gpu(event_loop, attrs)?;
        if gpu.window.fullscreen().is_some() {
            gpu.window.set_cursor_visible(false);
        }
        let interval = if self.opts.bench { SwapInterval::DontWait } else { SwapInterval::Wait(NonZeroU32::MIN) };
        let vsync = match gpu.surface.set_swap_interval(&gpu.context, interval) {
            Ok(()) => !self.opts.bench,
            Err(e) => {
                warn(&format!("could not enable vsync ({e}), using a frame limiter"));
                false
            }
        };

        let audio = if self.opts.sound && !offscreen && !self.opts.bench {
            game::audio()
        } else {
            audio::Audio::silent()
        };
        let rng = self.opts.seed.map(rng::Rng::new).unwrap_or_else(rng::Rng::from_time);
        let game = Game::new(&gpu.gl, audio, rng, self.opts.classic)?;
        let mut running = Running { game, gpu, last_frame: Instant::now(), vsync, fast_frames: 0, limiter: false };

        if let Some(dir) = self.opts.capture.clone() {
            capture(&mut running, &dir)?;
            event_loop.exit();
        } else if self.opts.bench {
            bench(&mut running)?;
            event_loop.exit();
        }
        self.running = Some(running);
        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_none()
            && let Err(e) = self.start(event_loop)
        {
            fatal(&e);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(r) = self.running.as_mut() else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) {
                    r.gpu.surface.resize(&r.gpu.context, w, h);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed && !event.repeat => {
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::Space) => r.game.spin(),
                    PhysicalKey::Code(KeyCode::F11 | KeyCode::KeyF) => r.toggle_fullscreen(),
                    PhysicalKey::Code(KeyCode::Escape) => {
                        if r.gpu.window.fullscreen().is_some() {
                            r.toggle_fullscreen();
                        } else {
                            event_loop.exit();
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, .. } => r.game.spin(),
            WindowEvent::RedrawRequested => r.frame(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(r) = &self.running {
            r.gpu.window.request_redraw();
        }
    }
}

/// Renders a scripted session into PNG files (for checking the visuals without a screen).
fn capture(r: &mut Running, dir: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let gl = &r.gpu.gl;
    let vp = Viewport { width: 1920, height: 1080 };
    let (fbo, tex) = unsafe {
        let tex = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            vp.width as i32,
            vp.height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        let fbo = gl.create_framebuffer()?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
        if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
            return Err("offscreen framebuffer is incomplete".into());
        }
        (fbo, tex)
    };

    let dt = 1.0 / 60.0;
    let game = &mut r.game;
    let shot = |game: &mut Game, name: &str| -> Result<(), String> {
        game.render(gl, vp);
        let mut pixels = vec![0u8; (vp.width * vp.height * 4) as usize];
        unsafe {
            gl.read_pixels(
                0,
                0,
                vp.width as i32,
                vp.height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
        }
        let row = (vp.width * 4) as usize;
        let flipped: Vec<u8> = pixels.chunks_exact(row).rev().flatten().copied().collect();
        let path = dir.join(format!("{name}.png"));
        image::save_buffer(&path, &flipped, vp.width, vp.height, image::ExtendedColorType::Rgba8)
            .map_err(|e| e.to_string())?;
        println!("wrote {}", path.display());
        Ok(())
    };
    let run = |game: &mut Game, seconds: f32| {
        for _ in 0..(seconds / dt).round() as usize {
            game.update(dt);
        }
    };

    run(game, 0.5);
    shot(game, "01-idle")?;
    game.spin();
    run(game, 0.6);
    shot(game, "02-spinning-fast")?;
    while game.debug().main_speed > 2.0 {
        game.update(dt);
    }
    while game.debug().flapper.abs() < 0.3 {
        game.update(dt);
    }
    shot(game, "03-flapper")?;

    let mut seen = Vec::new();
    for spin in 0..16 {
        if spin > 0 {
            game.spin();
        }
        let mut t = 0.0;
        while !(game.debug().number_shown && game.debug().prize_shown) && t < 40.0 {
            game.update(dt);
            t += dt;
        }
        let prize = game.debug().prize;
        if !seen.contains(&prize) {
            seen.push(prize);
            run(game, 0.3);
            shot(game, &format!("04-result-{prize:?}-a"))?;
            run(game, 1.0);
            shot(game, &format!("05-result-{prize:?}-b"))?;
        } else {
            run(game, 1.3);
        }
        if seen.len() >= 2 {
            break;
        }
    }
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.delete_framebuffer(fbo);
        gl.delete_texture(tex);
    }
    Ok(())
}

/// Renders frames as fast as possible during spins and reports frame times.
fn bench(r: &mut Running) -> Result<(), String> {
    let gl = &r.gpu.gl;
    let renderer = unsafe { gl.get_parameter_string(glow::RENDERER) };
    let version = unsafe { gl.get_parameter_string(glow::VERSION) };
    let Some(vp) = r.viewport() else { return Err("window has no size".into()) };
    let mut times = Vec::new();
    for i in 0..900 {
        if i % 300 == 0 {
            r.game.spin();
        }
        let t0 = Instant::now();
        r.game.update(1.0 / 60.0);
        r.game.render(gl, vp);
        unsafe { gl.finish() };
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
        let _ = r.gpu.surface.swap_buffers(&r.gpu.context);
    }
    times.drain(..30);
    times.sort_by(f64::total_cmp);
    let avg = times.iter().sum::<f64>() / times.len() as f64;
    let p95 = times[times.len() * 95 / 100];
    let max = times[times.len() - 1];
    let report = format!(
        "renderer: {renderer}\nversion: {version}\nviewport: {}x{}\nframe (update+render+finish): avg {avg:.2} ms, p95 {p95:.2} ms, max {max:.2} ms\n",
        vp.width, vp.height
    );
    print!("{report}");
    let _ = std::fs::write("chokladhjul-bench.txt", report);
    Ok(())
}

fn main() {
    let opts = Options::parse();
    let event_loop = EventLoop::new().unwrap_or_else(|e| fatal(&format!("could not start: {e}")));
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App { opts, running: None };
    if let Err(e) = event_loop.run_app(&mut app) {
        fatal(&format!("event loop error: {e}"));
    }
}
