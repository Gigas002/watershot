use std::fs;

use fontconfig::Fontconfig;
use image::DynamicImage;

use libwayshot::WayshotConnection;
use smithay_client_toolkit::{
    compositor::CompositorState,
    output::OutputState,
    reexports::client::{
        QueueHandle,
        globals::GlobalList,
        protocol::{wl_keyboard, wl_pointer},
    },
    registry::RegistryState,
    seat::{SeatState, pointer::ThemedPointer},
    shell::wlr_layer::LayerShell,
    shm::Shm,
};

use crate::{
    Config, Monitor, Rect, Selection, handles,
    rendering::Renderer,
    traits::{Contains, DistanceTo},
    types::{
        Args, ExitState, MonitorIdentification, RectangleSelection, SelectionModifier,
        SelectionState,
    },
    window::{
        CompositorBackend, FindWindowExt, InitializeBackend, WindowDescriptor,
        hyprland::HyprlandBackend,
    },
};

/// The main data worked on at runtime
pub struct RuntimeData {
    // Different wayland things
    pub registry_state: RegistryState,
    pub seat_state: SeatState,
    pub output_state: OutputState,
    pub compositor_state: CompositorState,
    pub layer_state: LayerShell,
    pub shm_state: Shm,

    // Devices
    pub keyboard: Option<wl_keyboard::WlKeyboard>,
    pub pointer: Option<wl_pointer::WlPointer>,
    pub themed_pointer: Option<ThemedPointer>,

    /// Combined area of all monitors
    pub area: Rect<i32>,
    /// The scale factor of the screenshot image
    pub scale_factor: f32,
    pub selection: Selection,
    pub monitors: Vec<Monitor>,
    pub config: Config,
    pub font: wgpu_text::glyph_brush::ab_glyph::FontArc,
    pub image: DynamicImage,
    pub exit: ExitState,

    pub instance: wgpu::Instance,
    pub device: wgpu::Device,
    pub adapter: wgpu::Adapter,
    pub queue: wgpu::Queue,

    pub renderer: Option<Renderer>,

    pub compositor_backend: Option<Box<dyn CompositorBackend>>,
    pub windows: Vec<WindowDescriptor>,
}

impl RuntimeData {
    pub fn get_preferred_backend() -> Option<Box<dyn CompositorBackend>> {
        HyprlandBackend::try_new().ok()
    }

    pub fn new(qh: &QueueHandle<Self>, globals: &GlobalList, mut args: Args) -> Self {
        let wayshot_connection = WayshotConnection::new().unwrap();
        let image = wayshot_connection.screenshot_all(false).unwrap();

        let config = Config::load().unwrap_or_default();

        let fc = Fontconfig::new().expect("Failed to init FontConfig");

        let fc_font = fc
            .find(&config.font_family, None)
            .expect("Failed to find font");

        let compositor_state =
            CompositorState::bind(globals, qh).expect("wl_compositor is not available");

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptionsBase {
                compatible_surface: None,
                ..Default::default()
            }))
            .unwrap();

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
                ..Default::default()
            },
            None,
        ))
        .unwrap();

        let compositor_backend = Self::get_preferred_backend();

        let mut selection = Selection::default();
        let mut windows = Vec::default();
        let mut exit = ExitState::None;

        if let Some(ref compositor_backend) = compositor_backend {
            (selection, windows, exit) = {
                let windows = compositor_backend.get_all_windows();

                let selection = {
                    match args.window_search.take() {
                        Some(search_param) => Selection::from_window(
                            windows.find_by_search_param(search_param).cloned(),
                        ),
                        _ => {
                            if args.window_under_cursor {
                                let mouse_pos = compositor_backend.get_mouse_position();
                                Selection::from_window(
                                    windows.find_by_position(&mouse_pos).cloned(),
                                )
                            } else if args.active_window {
                                Selection::from_window(compositor_backend.get_focused())
                            } else {
                                Selection::default()
                            }
                        }
                    }
                };

                if !args.auto_capture {
                    (selection, windows, ExitState::None)
                } else {
                    match selection.flattened() {
                        Selection::Rectangle(Some(rect_sel)) => (
                            selection,
                            windows,
                            ExitState::ExitWithSelection(rect_sel.extents.to_rect()),
                        ),
                        _ => {
                            // TODO: Auto-capture for monitors
                            (selection, windows, ExitState::None)
                        }
                    }
                }
            };
        }

        RuntimeData {
            registry_state: RegistryState::new(globals),
            seat_state: SeatState::new(globals, qh),
            output_state: OutputState::new(globals, qh),
            compositor_state,
            layer_state: LayerShell::bind(globals, qh).expect("layer shell is not available"),
            shm_state: Shm::bind(globals, qh).expect("wl_shm is not available"),
            selection,
            config,
            area: Rect::default(),
            monitors: Vec::new(),
            // Set later
            scale_factor: 0.0,
            image,
            keyboard: None,
            pointer: None,
            themed_pointer: None,
            exit,
            instance,
            adapter,
            device,
            queue,
            renderer: None,
            font: wgpu_text::glyph_brush::ab_glyph::FontArc::try_from_vec(
                fs::read(fc_font.path).expect("Failed to load font"),
            )
            .expect("Invalid font data"),
            compositor_backend,
            windows,
        }
    }

    pub fn draw(&mut self, identification: MonitorIdentification, qh: &QueueHandle<Self>) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };

        let monitor = match identification {
            MonitorIdentification::Layer(layer) => self
                .monitors
                .iter_mut()
                .find(|window| window.layer == layer)
                .unwrap(),
            MonitorIdentification::Surface(surface) => self
                .monitors
                .iter_mut()
                .find(|window| window.wl_surface == surface)
                .unwrap(),
        };

        if let Some(rendering) = &mut monitor.rendering {
            rendering.update_overlay_vertices(
                &monitor.rect,
                &monitor.wl_surface,
                &self.selection,
                &self.config,
                &self.queue,
            );
        }

        let surface_texture = monitor.surface.get_current_texture().unwrap();
        let texture_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());

        renderer.render(
            &mut encoder,
            &texture_view,
            monitor,
            &self.selection,
            &self.device,
            &self.queue,
        );

        self.queue.submit(Some(encoder.finish()));

        monitor
            .wl_surface
            .damage(0, 0, monitor.rect.width, monitor.rect.height);
        monitor.wl_surface.frame(qh, monitor.wl_surface.clone());
        surface_texture.present();
        monitor.wl_surface.commit();
    }

    pub fn process_selection_handles(
        rect_sel: &mut Option<RectangleSelection>,
        global_pos: (i32, i32),
        handle_radius: i32,
    ) -> SelectionState {
        if let Some(selection) = rect_sel {
            for (x, y, modifier) in handles!(selection.extents) {
                if global_pos.distance_to(&(*x, *y)) <= handle_radius {
                    selection.modifier = Some(*modifier);
                    selection.active = true;
                    return SelectionState::HandlesChanged;
                }
            }
            if selection.extents.to_rect().contains(&global_pos) {
                selection.modifier = Some(SelectionModifier::Center(
                    global_pos.0,
                    global_pos.1,
                    selection.extents,
                ));
                selection.active = true;
                return SelectionState::CenterChanged;
            }
        }

        SelectionState::Unchanged
    }
}
