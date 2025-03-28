use log::info;
use smithay_client_toolkit::{
    delegate_keyboard,
    reexports::client::{
        Connection, QueueHandle,
        protocol::{wl_keyboard, wl_surface},
    },
    seat::keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
};

use crate::{
    runtime_data::RuntimeData,
    types::{ExitState, Selection},
};

delegate_keyboard!(RuntimeData);

impl KeyboardHandler for RuntimeData {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        match event.keysym {
            // Exit without copying/saving
            Keysym::Escape => self.exit = ExitState::ExitOnly,
            // Switch selection mode
            Keysym::Tab => match &self.selection {
                Selection::Rectangle(_) => self.selection = Selection::Display(None),
                Selection::Display(_) => {
                    if self.compositor_backend.is_some() {
                        self.selection = Selection::Window(None)
                    } else {
                        self.selection = Selection::Rectangle(None)
                    }
                }
                Selection::Window(_) => self.selection = Selection::Rectangle(None),
            },
            // Exit with save if a valid selection exists
            Keysym::Return => {
                let flattened_selection = self.selection.flattened();
                match flattened_selection {
                    Selection::Rectangle(Some(selection)) => {
                        let mut rect = selection.extents.to_rect();
                        // Alter coordinate space so the rect can be used to crop from the original image
                        rect.x -= self.area.x;
                        rect.y -= self.area.y;

                        self.exit = ExitState::ExitWithSelection(rect)
                    }
                    Selection::Display(Some(selection)) => {
                        let monitor = self
                            .monitors
                            .iter()
                            .find(|monitor| monitor.wl_surface == selection.wl_surface)
                            .unwrap();

                        let mut rect = monitor.rect;

                        rect.x -= self.area.x;
                        rect.y -= self.area.y;

                        self.exit = ExitState::ExitWithSelection(rect)
                    }
                    Selection::Window(_) => unreachable!(
                        "Window selection should have been flattened into Rectangle selection"
                    ),
                    _ => (),
                }
            }
            _ => (),
        }
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        info!("Key release: {:?}", event);
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        _layout: u32,
    ) {
        info!("Update modifiers: {:?}", modifiers);
    }
}
