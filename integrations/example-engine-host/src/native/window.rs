use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use super::renderer::Renderer;
use fresco_example_engine::profile::{FrameInputs, camera::OrbitCamera};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use super::{Options, PreparedArtifact};

struct CompileEvent(Result<PreparedArtifact, String>);

struct GpuWindow {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    lost: Arc<Mutex<Option<String>>>,
}

enum DrawResult {
    Presented,
    Retry,
    Rebuild,
}

impl GpuWindow {
    async fn new(window: Arc<Window>, program: &PreparedArtifact) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| error.to_string())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|error| error.to_string())?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: fresco_example_engine::runtime::device::requested_limits(
                    &adapter.limits(),
                ),
                ..Default::default()
            })
            .await
            .map_err(|error| error.to_string())?;
        let lost = Arc::new(Mutex::new(None));
        let lost_callback = lost.clone();
        device.set_device_lost_callback(move |reason, message| {
            *lost_callback.lock().expect("device-loss state") =
                Some(format!("{reason:?}: {message}"));
        });
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("adapter cannot present to this window")?;
        config.width = size.width;
        config.height = size.height;
        let renderer = Renderer::prepare(&device, &queue, config.format, program).await?;
        if size.width != 0 && size.height != 0 {
            surface.configure(&device, &config);
        }
        println!(
            "Rendering `{}` on {} ({:?}).",
            program.entry,
            adapter.get_info().name,
            adapter.get_info().backend
        );
        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            renderer,
            lost,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.config.width = size.width;
        self.config.height = size.height;
        if size.width != 0 && size.height != 0 {
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn draw(
        &mut self,
        time: f32,
        delta_time: f32,
        camera: OrbitCamera,
    ) -> Result<DrawResult, String> {
        if let Some(reason) = self.lost.lock().expect("device-loss state").take() {
            eprintln!("Device lost; rebuilding resources: {reason}");
            return Ok(DrawResult::Rebuild);
        }
        if self.config.width == 0 || self.config.height == 0 {
            return Ok(DrawResult::Retry);
        }
        let (texture, reconfigure) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => (texture, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => (texture, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(DrawResult::Retry);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(DrawResult::Retry);
            }
            wgpu::CurrentSurfaceTexture::Lost => return Ok(DrawResult::Rebuild),
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("surface acquisition failed GPU validation".into());
            }
        };
        let view = texture.texture.create_view(&Default::default());
        self.renderer.render(
            &self.device,
            FrameInputs {
                time,
                delta_time,
                physical_size: [self.config.width, self.config.height],
            },
            camera,
            &view,
        )?;
        drop(view);
        self.window.pre_present_notify();
        self.queue.present(texture);
        if reconfigure {
            self.surface.configure(&self.device, &self.config);
        }
        Ok(DrawResult::Presented)
    }
}

type Stamp = Vec<(PathBuf, SystemTime, u64)>;

fn source_stamp(options: &Options, asset_paths: &[PathBuf]) -> Result<Stamp, String> {
    fn visit(path: &Path, stamp: &mut Stamp) -> Result<(), String> {
        for item in
            std::fs::read_dir(path).map_err(|error| format!("watch {}: {error}", path.display()))?
        {
            let item = item.map_err(|error| error.to_string())?;
            let kind = item.file_type().map_err(|error| error.to_string())?;
            let name = item.file_name();
            if kind.is_dir() && !matches!(name.to_str(), Some(".git" | "target" | "node_modules")) {
                visit(&item.path(), stamp)?;
            } else if kind.is_file() && item.path().extension().is_some_and(|ext| ext == "fr") {
                let metadata = item.metadata().map_err(|error| error.to_string())?;
                stamp.push((
                    item.path(),
                    metadata.modified().map_err(|error| error.to_string())?,
                    metadata.len(),
                ));
            }
        }
        Ok(())
    }
    let mut roots = Vec::new();
    if let Some(source) = &options.source {
        let source = std::fs::canonicalize(source).map_err(|error| error.to_string())?;
        roots.push(
            source
                .parent()
                .ok_or("source has no parent directory")?
                .to_path_buf(),
        );
    }
    if let Some(engine) = &options.engine_dir {
        roots.push(engine.clone());
    }
    let mut stamp = Vec::new();
    for path in asset_paths {
        let metadata = std::fs::metadata(path)
            .map_err(|error| format!("watch asset {}: {error}", path.display()))?;
        stamp.push((
            path.clone(),
            metadata.modified().map_err(|error| error.to_string())?,
            metadata.len(),
        ));
    }
    for root in roots {
        visit(&root, &mut stamp)?;
    }
    if let Some(path) = &options.params_file {
        let metadata = std::fs::metadata(path)
            .map_err(|error| format!("watch {}: {error}", path.display()))?;
        stamp.push((
            path.clone(),
            metadata.modified().map_err(|error| error.to_string())?,
            metadata.len(),
        ));
    }
    stamp.sort();
    stamp.dedup();
    Ok(stamp)
}

struct App {
    camera: OrbitCamera,
    dragging: bool,
    cursor: Option<[f64; 2]>,
    options: Options,
    program: PreparedArtifact,
    pending: Option<PreparedArtifact>,
    gpu: Option<GpuWindow>,
    proxy: EventLoopProxy<CompileEvent>,
    error: Option<String>,
    compiling: bool,
    paused: bool,
    time: f64,
    last_frame: Instant,
    next_frame: Instant,
    last_watch: Instant,
    stamp: Option<Stamp>,
    presented: u32,
    started: Instant,
}

impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.error = Some(error);
        event_loop.exit();
    }

    fn reload(&mut self) {
        if self.compiling {
            return;
        }
        let options = self.options.clone();
        let proxy = self.proxy.clone();
        match std::thread::Builder::new()
            .name("fresco-reload".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(move || {
                let result = super::compile(&options);
                // Closing the window while compilation runs is normal.
                let _ = proxy.send_event(CompileEvent(result));
            }) {
            Ok(_) => self.compiling = true,
            Err(error) => eprintln!("Cannot start compiler worker: {error}"),
        }
    }
    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        if self
            .options
            .frames
            .is_some_and(|limit| self.presented >= limit)
        {
            return;
        }
        let Some(gpu) = &mut self.gpu else {
            return;
        };
        let now = Instant::now();
        let delta = if self.paused {
            0.0
        } else {
            now.duration_since(self.last_frame).as_secs_f64()
        };
        self.last_frame = now;
        self.time += delta;
        match gpu.draw(self.time as f32, delta as f32, self.camera) {
            Ok(DrawResult::Presented) => {
                self.presented = self.presented.saturating_add(1);
                if self
                    .options
                    .frames
                    .is_some_and(|limit| self.presented >= limit)
                {
                    println!("Presented {} frame(s).", self.presented);
                    event_loop.exit();
                }
            }
            Ok(DrawResult::Retry) => {}
            Ok(DrawResult::Rebuild) => {
                let window = gpu.window.clone();
                // Release the old swapchain before creating another for this window.
                self.gpu = None;
                match pollster::block_on(GpuWindow::new(window, &self.program)) {
                    Ok(rebuilt) => self.gpu = Some(rebuilt),
                    Err(error) => self.fail(event_loop, format!("GPU rebuild failed: {error}")),
                }
            }
            Err(error) => self.fail(event_loop, error),
        }
    }
}

impl ApplicationHandler<CompileEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        let result = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("Fresco example — Space: pause, Home: reset, R: reload")
                    .with_inner_size(PhysicalSize::new(960, 640))
                    .with_visible(!self.options.hidden),
            )
            .map_err(|error| error.to_string())
            .and_then(|window| {
                let window = Arc::new(window);
                if let Some(program) = self.pending.take() {
                    match pollster::block_on(GpuWindow::new(window.clone(), &program)) {
                        Ok(gpu) => {
                            self.program = program;
                            return Ok(gpu);
                        }
                        Err(error) => eprintln!(
                            "Reload failed after resume; keeping the last valid program: {error}"
                        ),
                    }
                }
                pollster::block_on(GpuWindow::new(window, &self.program))
            });
        match result {
            Ok(gpu) => {
                self.gpu = Some(gpu);
                self.last_frame = Instant::now();
            }
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.gpu = None;
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, CompileEvent(result): CompileEvent) {
        self.compiling = false;
        let result = result.and_then(|program| {
            if let Some(gpu) = &mut self.gpu {
                let candidate = pollster::block_on(Renderer::prepare(
                    &gpu.device,
                    &gpu.queue,
                    gpu.config.format,
                    &program,
                ))?;
                gpu.renderer = candidate;
                gpu.window
                    .set_title("Fresco example — Space: pause, Home: reset, R: reload");
            }
            println!("Installed {} `{}`.", program.kind.label(), program.entry);
            self.program = program;
            Ok(())
        });
        if let Err(error) = result {
            eprintln!("Reload failed; keeping the last valid program:\n{error}");
            if let Some(gpu) = &self.gpu {
                gpu.window
                    .set_title("Fresco example — reload failed (see console)");
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(gpu) = &mut self.gpu else {
            return;
        };
        if gpu.window.id() != id {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => gpu.resize(size),
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.dragging = state == ElementState::Pressed;
            }
            WindowEvent::CursorMoved { position, .. } => {
                let current = [position.x, position.y];
                if self.dragging
                    && let Some(previous) = self.cursor
                {
                    let scale = gpu.window.scale_factor();
                    let delta =
                        std::array::from_fn(|i| ((current[i] - previous[i]) / scale) as f32);
                    if let Err(error) = self.camera.drag(delta) {
                        eprintln!("Camera input rejected: {error}");
                    }
                }
                self.cursor = Some(current);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // Positive winit wheel input scrolls up; positive camera input zooms out.
                let pixels = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 40.0,
                    MouseScrollDelta::PixelDelta(position) => {
                        -(position.y / gpu.window.scale_factor()) as f32
                    }
                };
                if let Err(error) = self.camera.zoom(pixels) {
                    eprintln!("Camera input rejected: {error}");
                }
            }
            WindowEvent::Focused(false) | WindowEvent::CursorLeft { .. } => {
                self.dragging = false;
                self.cursor = None;
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                match event.logical_key {
                    Key::Named(NamedKey::Escape) => event_loop.exit(),
                    Key::Named(NamedKey::Space) => {
                        self.paused = !self.paused;
                        self.last_frame = Instant::now();
                    }
                    Key::Named(NamedKey::Home) => {
                        if let Err(error) = gpu.renderer.reset() {
                            eprintln!("Playback reset failed: {error}");
                            return;
                        }
                        self.time = 0.0;
                        self.last_frame = Instant::now();
                    }
                    Key::Character(key) if key.eq_ignore_ascii_case("r") => self.reload(),
                    Key::Character(key) if key.eq_ignore_ascii_case("c") => {
                        self.camera = OrbitCamera::default();
                    }
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if self.options.frames.is_some()
            && now.duration_since(self.started) > Duration::from_secs(30)
        {
            self.fail(
                event_loop,
                "timed out waiting for the requested presented frames".into(),
            );
            return;
        }
        if self.options.watch
            && !self.compiling
            && now.duration_since(self.last_watch) >= Duration::from_millis(500)
        {
            self.last_watch = now;
            match source_stamp(&self.options, &self.program.asset_paths) {
                Ok(stamp) if self.stamp.as_ref() != Some(&stamp) => {
                    self.stamp = Some(stamp);
                    self.reload();
                }
                Ok(_) => {}
                Err(error) => eprintln!("{error}"),
            }
        }
        if now >= self.next_frame {
            if self.options.hidden {
                // Hidden Windows windows may not deliver RedrawRequested events.
                // Still acquire, render, and present real surface frames.
                self.redraw(event_loop);
            } else if let Some(gpu) = &self.gpu {
                gpu.window.request_redraw();
            }
            self.next_frame = now + Duration::from_millis(16);
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

pub(super) fn run(options: Options, program: PreparedArtifact) -> Result<(), String> {
    let event_loop = EventLoop::<CompileEvent>::with_user_event()
        .build()
        .map_err(|error| error.to_string())?;
    let stamp = if options.watch {
        Some(source_stamp(&options, &program.asset_paths)?)
    } else {
        None
    };
    let now = Instant::now();
    let mut app = App {
        camera: OrbitCamera::default(),
        dragging: false,
        cursor: None,
        options,
        program,
        pending: None,
        gpu: None,
        proxy: event_loop.create_proxy(),
        error: None,
        compiling: false,
        paused: false,
        time: 0.0,
        last_frame: now,
        next_frame: now,
        last_watch: now,
        stamp,
        presented: 0,
        started: now,
    };
    event_loop
        .run_app(&mut app)
        .map_err(|error| error.to_string())?;
    match app.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
