//! Canvas ownership and asynchronous installation; rendering stays in the shared runtime.
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use fresco_artifact::ManifestRoot;
use fresco_example_engine::profile::{FrameInputs, camera::OrbitCamera};
use fresco_example_engine::runtime::depth::DepthTarget;
mod renderer;
use renderer::Renderer;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

struct State {
    frame_generation: u64,
    camera: OrbitCamera,
    canvas: HtmlCanvasElement,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: Option<Renderer>,
    depth: Option<DepthTarget>,
    requested_size: [u32; 2],
    resize_generation: u64,
    resizing: bool,
    generation: u64,
    lost: Arc<Mutex<Option<String>>>,
}

impl State {
    fn set_size(&mut self, size: [u32; 2]) {
        if [self.config.width, self.config.height] == size {
            return;
        }
        self.canvas.set_width(size[0]);
        self.canvas.set_height(size[1]);
        self.config.width = size[0];
        self.config.height = size[1];
        if !size.contains(&0) {
            self.surface.configure(&self.device, &self.config);
        }
    }
}

/// An independent rendering WASM instance. It contains no Fresco compiler or editor.
#[wasm_bindgen]
pub struct BrowserEngine {
    state: Rc<RefCell<State>>,
}

/// CPU-only camera state can outlive a rendering device.
#[wasm_bindgen]
#[derive(Default)]
pub struct BrowserCamera {
    camera: OrbitCamera,
}

#[wasm_bindgen]
impl BrowserCamera {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn drag(&mut self, x: f32, y: f32) -> Result<(), JsValue> {
        self.camera.drag([x, y]).map_err(js_error)
    }

    pub fn zoom(&mut self, pixels: f32) -> Result<(), JsValue> {
        self.camera.zoom(pixels).map_err(js_error)
    }

    pub fn reset(&mut self) {
        self.camera = OrbitCamera::default();
    }
}

#[wasm_bindgen]
impl BrowserEngine {
    pub fn buffer_views(&self) -> Result<String, JsValue> {
        match self.state.borrow().renderer.as_ref() {
            Some(Renderer::Mesh(renderer, _)) => {
                serde_json::to_string(renderer.buffer_views()).map_err(js_error)
            }
            Some(Renderer::Particles(renderer)) => {
                serde_json::to_string(renderer.buffer_views()).map_err(js_error)
            }
            Some(Renderer::Canvas(_)) | None => Ok("[]".into()),
        }
    }
    pub fn set_buffer_view(&self, id: String) -> Result<(), JsValue> {
        match self.state.borrow().renderer.as_ref() {
            Some(Renderer::Mesh(renderer, _)) => renderer.set_buffer_view(&id).map_err(js_error),
            Some(Renderer::Particles(renderer)) => renderer.set_buffer_view(&id).map_err(js_error),
            Some(Renderer::Canvas(_)) | None => {
                Err(js_error("buffer views require an installed 3-D scene"))
            }
        }
    }
    pub fn supports_lighting_environment(&self) -> bool {
        match self.state.borrow().renderer.as_ref() {
            Some(Renderer::Mesh(renderer, _)) => renderer.supports_lighting_environment(),
            Some(Renderer::Particles(renderer)) => renderer.supports_lighting_environment(),
            Some(Renderer::Canvas(_)) | None => false,
        }
    }
    pub fn set_lighting_environment(&self, name: String) -> Result<(), JsValue> {
        let environment =
            serde_json::from_value(serde_json::Value::String(name)).map_err(js_error)?;
        let mut state = self.state.borrow_mut();
        match state.renderer.as_mut() {
            Some(Renderer::Mesh(renderer, _)) => renderer
                .set_lighting_environment(environment)
                .map_err(js_error),
            Some(Renderer::Particles(renderer)) => renderer
                .set_lighting_environment(environment)
                .map_err(js_error),
            Some(Renderer::Canvas(_)) | None => {
                Err(js_error("lighting requires an installed 3-D scene"))
            }
        }
    }
    /// Replace all point lights in an installed forward+ mesh. Invalid edits do
    /// not mutate GPU buffers. Shader reinstallation restores the demo lights.
    pub fn set_point_lights(&self, json: String) -> Result<(), JsValue> {
        let lights: Vec<fresco_example_engine::runtime::forward_plus::PointLight> =
            serde_json::from_str(&json).map_err(js_error)?;
        let mut state = self.state.borrow_mut();
        match state.renderer.as_mut() {
            Some(Renderer::Mesh(renderer, _)) => {
                renderer.set_point_lights(&lights).map_err(js_error)
            }
            _ => Err(js_error("point lights require an installed forward+ mesh")),
        }
    }

    pub fn set_camera(&self, camera: &BrowserCamera) {
        self.state.borrow_mut().camera = camera.camera;
    }
    pub async fn create(canvas: HtmlCanvasElement) -> Result<BrowserEngine, JsValue> {
        console_error_panic_hook::set_once();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(js_error)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(js_error)?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: fresco_example_engine::runtime::device::requested_limits(
                    &adapter.limits(),
                ),
                ..Default::default()
            })
            .await
            .map_err(js_error)?;
        let lost = Arc::new(Mutex::new(None));
        let callback = lost.clone();
        device.set_device_lost_callback(move |reason, message| {
            *callback.lock().expect("device-loss state") = Some(format!(
                "GPU device lost ({reason:?}): {message}; rebuild the browser engine"
            ));
        });
        let mut config = surface
            .get_default_config(&adapter, canvas.width().max(1), canvas.height().max(1))
            .ok_or_else(|| js_error("this adapter cannot present to the canvas"))?;
        config.width = canvas.width();
        config.height = canvas.height();
        if config.width > device.limits().max_texture_dimension_2d
            || config.height > device.limits().max_texture_dimension_2d
        {
            return Err(js_error("canvas dimensions exceed the device limit"));
        }
        if config.width != 0 && config.height != 0 {
            surface.configure(&device, &config);
        }
        Ok(Self {
            state: Rc::new(RefCell::new(State {
                frame_generation: 0,
                camera: OrbitCamera::default(),
                canvas,
                surface,
                device,
                queue,
                requested_size: [config.width, config.height],
                resize_generation: 0,
                resizing: false,
                depth: None,
                config,
                renderer: None,
                generation: 0,
                lost,
            })),
        })
    }

    /// Render calls may continue while preparation awaits GPU validation. Only the
    /// newest successful request installs; failures preserve the active renderer.
    pub fn install(&self, wgsl: String, manifest_json: String, entry: String) -> js_sys::Promise {
        self.install_with_assets(wgsl, manifest_json, entry, js_sys::Map::new(), None, None)
    }

    /// Encoded PNG/JPEG bytes keyed by the texture name in the selected entry.
    pub fn install_with_assets(
        &self,
        wgsl: String,
        manifest_json: String,
        entry: String,
        assets: js_sys::Map,
        parameters_json: Option<String>,
        mesh: Option<String>,
    ) -> js_sys::Promise {
        let options = parameters_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map(|parameters| InstallOptions {
                mesh,
                parameters,
                variant: None,
            })
            .map_err(|error| error.to_string());
        self.install_candidate(wgsl, manifest_json, entry, assets, options)
    }

    /// Install with a JSON object containing optional mesh, parameters, and variant bindings.
    pub fn install_with_options(
        &self,
        wgsl: String,
        manifest_json: String,
        entry: String,
        assets: js_sys::Map,
        options_json: String,
    ) -> js_sys::Promise {
        let options = serde_json::from_str(&options_json).map_err(|error| error.to_string());
        self.install_candidate(wgsl, manifest_json, entry, assets, options)
    }

    /// Cancel pending preparation without discarding the currently visible program.
    pub fn cancel_pending(&self) -> Result<(), JsValue> {
        let mut state = self.state.borrow_mut();
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or_else(|| js_error("installation generation exhausted"))?;
        Ok(())
    }

    pub fn parameter_values_json(&self) -> Result<String, JsValue> {
        let state = self.state.borrow();
        let renderer = state
            .renderer
            .as_ref()
            .ok_or_else(|| js_error("no installed renderer"))?;
        serde_json::to_string(&renderer.values()).map_err(js_error)
    }

    pub fn update_parameters(&self, json: String) -> js_sys::Promise {
        let generation = {
            let mut state = self.state.borrow_mut();
            let Some(generation) = state.generation.checked_add(1) else {
                return js_sys::Promise::reject(&js_error("installation generation exhausted"));
            };
            state.generation = generation;
            generation
        };
        let state = self.state.clone();
        wasm_bindgen_futures::future_to_promise(async move {
            let updates = serde_json::from_str(&json).map_err(js_error)?;
            let pending = {
                let mut state = state.borrow_mut();
                if generation != state.generation {
                    return Ok(JsValue::FALSE);
                }
                let renderer = state
                    .renderer
                    .as_mut()
                    .ok_or_else(|| js_error("no installed renderer"))?;
                match renderer {
                    Renderer::Particles(renderer) => {
                        renderer.update_parameters(&updates).map_err(js_error)?;
                        return Ok(JsValue::TRUE);
                    }
                    Renderer::Mesh(renderer, _) => {
                        renderer.update_parameters(&updates).map_err(js_error)?;
                        return Ok(JsValue::TRUE);
                    }
                    Renderer::Canvas(renderer) => renderer
                        .begin_parameter_update(&updates)
                        .map_err(js_error)?,
                }
            };
            let prepared = pending.prepare().await.map_err(js_error)?;
            let mut state = state.borrow_mut();
            if generation != state.generation {
                return Ok(JsValue::FALSE);
            }
            let renderer = state
                .renderer
                .as_mut()
                .ok_or_else(|| js_error("no installed renderer"))?;
            let Renderer::Canvas(renderer) = renderer else {
                return Err(js_error(
                    "canvas parameter candidate no longer matches the installed renderer",
                ));
            };
            renderer
                .apply_parameter_update(prepared)
                .map_err(js_error)?;
            Ok(JsValue::TRUE)
        })
    }

    /// Canvas resizing commits synchronously; mesh resizing returns a promise
    /// after validating a replacement depth target. Rendering keeps the old size
    /// until the newest resize succeeds.
    pub fn resize(&self, width: u32, height: u32) -> Result<js_sys::Promise, JsValue> {
        let mut current = self.state.borrow_mut();
        let max = current.device.limits().max_texture_dimension_2d;
        if width > max || height > max {
            return Err(js_error(format!(
                "canvas dimensions exceed device limit {max}"
            )));
        }
        let size = [width, height];
        if current.requested_size == size
            && (current.resizing || [current.config.width, current.config.height] == size)
        {
            return Ok(js_sys::Promise::resolve(&JsValue::FALSE));
        }
        current.resize_generation = current
            .resize_generation
            .checked_add(1)
            .ok_or_else(|| js_error("resize generation exhausted"))?;
        let generation = current.resize_generation;
        current.requested_size = size;
        if size.contains(&0)
            || current
                .renderer
                .as_ref()
                .is_none_or(|renderer| !renderer.is_mesh())
        {
            current.resizing = false;
            current.depth = None;
            current.set_size(size);
            return Ok(js_sys::Promise::resolve(&JsValue::TRUE));
        }
        current.resizing = true;
        let device = current.device.clone();
        drop(current);
        let state = self.state.clone();
        Ok(wasm_bindgen_futures::future_to_promise(async move {
            let result = DepthTarget::prepare(&device, size).await;
            let mut state = state.borrow_mut();
            if state.resize_generation != generation {
                return Ok(JsValue::FALSE);
            }
            state.resizing = false;
            let depth = result.map_err(js_error)?;
            state.set_size(size);
            if state.renderer.as_ref().is_some_and(Renderer::is_mesh) {
                state.depth = depth;
            }
            Ok(JsValue::TRUE)
        }))
    }

    /// Snapshot committed scheduler metadata without exposing mutable engine state.
    pub fn particle_slots_json(&self) -> Result<String, JsValue> {
        let state = self.state.borrow();
        let Some(Renderer::Particles(renderer)) = &state.renderer else {
            return Err(js_error("particle renderer required"));
        };
        let pool = renderer
            .pool()
            .ok_or_else(|| js_error("managed particles required"))?;
        serde_json::to_string(&serde_json::json!({
            "capacity": pool.capacity(),
            "dropped": pool.dropped(),
            "words": pool.commands().iter().flatten().copied().collect::<Vec<_>>(),
        }))
        .map_err(js_error)
    }

    /// Read back the state submitted before this call. This does not advance time.
    /// The returned copy remains valid across later shader installs and resets.
    pub fn particle_state_bytes(&self) -> js_sys::Promise {
        let staging = {
            let state = self.state.borrow();
            let Some(Renderer::Particles(renderer)) = &state.renderer else {
                return js_sys::Promise::reject(&js_error("particle renderer required"));
            };
            renderer.copy_state_for_readback()
        };
        js_sys::Promise::new(&mut |resolve, reject| {
            let buffer = staging.clone();
            staging
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let bytes = result.map_err(js_error).and_then(|()| {
                        let view = buffer.slice(..).get_mapped_range().map_err(js_error)?;
                        Ok(js_sys::Uint8Array::from(view.as_ref()))
                    });
                    buffer.unmap();
                    buffer.destroy();
                    match bytes {
                        Ok(bytes) => {
                            let _ = resolve.call1(&JsValue::UNDEFINED, &bytes);
                        }
                        Err(error) => {
                            let _ = reject.call1(&JsValue::UNDEFINED, &error);
                        }
                    }
                });
        })
    }

    pub fn reset_playback(&self) -> Result<(), JsValue> {
        let mut state = self.state.borrow_mut();
        state.frame_generation = state
            .frame_generation
            .checked_add(1)
            .ok_or_else(|| js_error("frame generation exhausted"))?;
        if let Some(Renderer::Particles(renderer)) = &mut state.renderer {
            renderer.reset().map_err(js_error)?;
        }
        Ok(())
    }

    pub fn render_async(&self, time: f32, delta_time: f32) -> js_sys::Promise {
        if !matches!(self.state.borrow().renderer, Some(Renderer::Particles(_))) {
            return match self.render(time, delta_time) {
                Ok(drawn) => js_sys::Promise::resolve(&JsValue::from_bool(drawn)),
                Err(error) => js_sys::Promise::reject(&error),
            };
        }
        let snapshot = (|| {
            let mut state = self.state.borrow_mut();
            state.frame_generation = state
                .frame_generation
                .checked_add(1)
                .ok_or_else(|| js_error("frame generation exhausted"))?;
            let Some(Renderer::Particles(renderer)) = &state.renderer else {
                unreachable!("checked particle renderer");
            };
            let inputs = state.camera.scene(
                fresco_example_engine::profile::preview::PreviewShape::Sphere,
                FrameInputs {
                    time,
                    delta_time,
                    physical_size: [state.config.width, state.config.height],
                },
            );
            let pending = renderer.begin_frame(&inputs).map_err(js_error)?;
            Ok::<_, JsValue>((
                pending,
                state.generation,
                state.resize_generation,
                state.frame_generation,
            ))
        })();
        let (pending, generation, resize_generation, frame_generation) = match snapshot {
            Ok(value) => value,
            Err(error) => return js_sys::Promise::reject(&error),
        };
        let Some(pending) = pending else {
            return js_sys::Promise::resolve(&JsValue::FALSE);
        };
        let state = self.state.clone();
        wasm_bindgen_futures::future_to_promise(async move {
            let prepared = pending.prepare().await;
            let mut state = state.borrow_mut();
            if state.generation != generation
                || state.resize_generation != resize_generation
                || state.frame_generation != frame_generation
            {
                return Ok(JsValue::FALSE);
            }
            let drawn = render_state(
                &mut state,
                time,
                delta_time,
                Some(prepared.map_err(js_error)?),
            )?;
            Ok(JsValue::from_bool(drawn))
        })
    }

    pub fn render(&self, time: f32, delta_time: f32) -> Result<bool, JsValue> {
        render_state(&mut self.state.borrow_mut(), time, delta_time, None)
    }
}

fn render_state(
    state: &mut State,
    time: f32,
    delta_time: f32,
    prepared: Option<fresco_example_engine::runtime::particles::PreparedParticleFrame>,
) -> Result<bool, JsValue> {
    if let Some(reason) = state.lost.lock().expect("device-loss state").as_ref() {
        return Err(js_error(reason));
    }
    if state.renderer.is_none() || state.config.width == 0 || state.config.height == 0 {
        return Ok(false);
    }
    let (texture, reconfigure) = match state.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(texture) => (texture, false),
        wgpu::CurrentSurfaceTexture::Suboptimal(texture) => (texture, true),
        wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
            return Ok(false);
        }
        wgpu::CurrentSurfaceTexture::Outdated => {
            state.surface.configure(&state.device, &state.config);
            return Ok(false);
        }
        wgpu::CurrentSurfaceTexture::Lost => {
            return Err(js_error("canvas surface lost; rebuild the browser engine"));
        }
        wgpu::CurrentSurfaceTexture::Validation => {
            return Err(js_error("canvas surface failed GPU validation"));
        }
    };
    let view = texture.texture.create_view(&Default::default());
    let physical_size = [state.config.width, state.config.height];
    let State {
        renderer,
        depth,
        camera,
        ..
    } = state;
    if let Some(prepared) = prepared {
        let Some(Renderer::Particles(particles)) = renderer.as_mut() else {
            return Err(js_error(
                "prepared particle frame no longer matches renderer",
            ));
        };
        let depth = depth
            .as_ref()
            .filter(|d| d.size() == physical_size)
            .ok_or_else(|| js_error("particle depth dimensions do not match frame"))?;
        particles
            .render_prepared(prepared, &view, depth.view())
            .map_err(js_error)?;
    } else {
        renderer
            .as_mut()
            .expect("renderer checked")
            .render(
                FrameInputs {
                    time,
                    delta_time,
                    physical_size,
                },
                *camera,
                &view,
                depth.as_ref(),
            )
            .map_err(js_error)?;
    }
    drop(view);
    state.queue.present(texture);
    if reconfigure {
        state.surface.configure(&state.device, &state.config);
    }
    Ok(true)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallOptions {
    mesh: Option<String>,
    parameters: Option<serde_json::Map<String, serde_json::Value>>,
    variant: Option<fresco_example_engine::runtime::canvas_variant::VariantSelection>,
}

impl BrowserEngine {
    fn install_candidate(
        &self,
        wgsl: String,
        manifest_json: String,
        entry: String,
        assets: js_sys::Map,
        options: Result<InstallOptions, String>,
    ) -> js_sys::Promise {
        let (device, queue, format, generation) = {
            let mut state = self.state.borrow_mut();
            let Some(generation) = state.generation.checked_add(1) else {
                return js_sys::Promise::reject(&js_error("installation generation exhausted"));
            };
            state.generation = generation;
            (
                state.device.clone(),
                state.queue.clone(),
                state.config.format,
                generation,
            )
        };
        // Clone the state before returning: no exported Rust borrow spans an await,
        // so rendering and disposal remain safe while preparation is pending.
        let state = self.state.clone();
        wasm_bindgen_futures::future_to_promise(async move {
            let options = options.map_err(js_error)?;
            let shape = options
                .mesh
                .as_deref()
                .unwrap_or("sphere")
                .parse()
                .map_err(js_error)?;
            let manifest: ManifestRoot = serde_json::from_str(&manifest_json).map_err(js_error)?;
            let definitions = renderer::textures(&manifest, &entry).map_err(js_error)?;
            let mut encoded = std::collections::BTreeMap::new();
            for pair in assets.entries() {
                let pair = js_sys::Array::from(&pair?);
                let name = pair
                    .get(0)
                    .as_string()
                    .ok_or_else(|| js_error("asset keys must be texture names"))?;
                let bytes = pair
                    .get(1)
                    .dyn_into::<js_sys::Uint8Array>()
                    .map_err(|_| js_error("asset values must be Uint8Array image bytes"))?;
                encoded.insert(name, bytes.to_vec());
            }
            let textures =
                fresco_example_engine::assets::resolve(definitions, &encoded).map_err(js_error)?;
            let parameters = options.parameters;
            let candidate = Renderer::prepare(
                device.clone(),
                queue,
                format,
                renderer::Request {
                    shape,
                    wgsl: &wgsl,
                    manifest: &manifest,
                    entry: &entry,
                    textures: &textures,
                    parameters: parameters.as_ref(),
                    variant: options.variant.as_ref(),
                },
            )
            .await
            .map_err(js_error)?;
            // A resize can occur during shader preparation. Prepare depth for
            // the latest requested dimensions before committing a mesh candidate.
            let depth = if candidate.is_mesh() {
                loop {
                    let size = {
                        let current = state.borrow();
                        if generation != current.generation {
                            return Ok(JsValue::FALSE);
                        }
                        current.requested_size
                    };
                    let depth = DepthTarget::prepare(&device, size)
                        .await
                        .map_err(js_error)?;
                    if state.borrow().requested_size == size {
                        break depth;
                    }
                }
            } else {
                None
            };
            let mut state = state.borrow_mut();
            if generation != state.generation {
                return Ok(JsValue::FALSE);
            }
            state.resize_generation = state
                .resize_generation
                .checked_add(1)
                .ok_or_else(|| js_error("resize generation exhausted"))?;
            state.resizing = false;
            let size = state.requested_size;
            state.set_size(size);
            state.depth = depth;
            state.frame_generation = state
                .frame_generation
                .checked_add(1)
                .ok_or_else(|| js_error("frame generation exhausted"))?;
            state.renderer = Some(candidate);
            Ok(JsValue::TRUE)
        })
    }
}
