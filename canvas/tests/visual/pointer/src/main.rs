//! Minimal deterministic pointer for the visual regression suite.
//!
//! Usage: wlr-virtual-pointer click <x> <y> <left|right> --extent <width>x<height>
//! It attaches a virtual pointer to the existing wlroots seat and emits one
//! absolute motion + press/release pair in the compositor's coordinate space.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wayland_client::{
    Connection, Dispatch, QueueHandle,
    delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_output::WlOutput,
        wl_pointer::ButtonState,
        wl_registry::{self, WlRegistry},
        wl_seat::WlSeat,
    },
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

struct State;

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore WlSeat);
delegate_noop!(State: ignore WlOutput);
delegate_noop!(State: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ignore ZwlrVirtualPointerV1);

fn now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u32
}

fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("wlr-virtual-pointer: {message}");
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 6 || args[0] != "click" || args[4] != "--extent" {
        fail("usage: click <x> <y> <left|right> --extent <width>x<height>");
    }
    let x: u32 = args[1].parse().unwrap_or_else(|_| fail("x must be an integer"));
    let y: u32 = args[2].parse().unwrap_or_else(|_| fail("y must be an integer"));
    let button = match args[3].as_str() {
        "left" => 0x110,
        "right" => 0x111,
        other => fail(format!("unknown button {other}")),
    };
    let (extent_x, extent_y) = args[5]
        .split_once('x')
        .unwrap_or_else(|| fail("extent must be WIDTHxHEIGHT"));
    let extent_x: u32 = extent_x.parse().unwrap_or_else(|_| fail("bad extent width"));
    let extent_y: u32 = extent_y.parse().unwrap_or_else(|_| fail("bad extent height"));

    let connection = Connection::connect_to_env().unwrap_or_else(|error| fail(error));
    let (globals, mut queue) =
        registry_queue_init::<State>(&connection).unwrap_or_else(|error| fail(error));
    let qh = queue.handle();
    let mut state = State;

    let seat = globals
        .bind::<WlSeat, _, _>(&qh, 1..=7, ())
        .unwrap_or_else(|error| fail(format!("no wl_seat: {error}")));
    let output = globals
        .bind::<WlOutput, _, _>(&qh, 1..=4, ())
        .unwrap_or_else(|error| fail(format!("no wl_output: {error}")));
    let manager = globals
        .bind::<ZwlrVirtualPointerManagerV1, _, _>(&qh, 1..=2, ())
        .unwrap_or_else(|error| fail(format!("no virtual pointer manager: {error}")));

    let pointer = manager.create_virtual_pointer_with_output(Some(&seat), Some(&output), &qh, ());

    pointer.motion_absolute(now(), x, y, extent_x, extent_y);
    pointer.frame();
    queue.roundtrip(&mut state).unwrap_or_else(|error| fail(error));
    std::thread::sleep(Duration::from_millis(80));

    pointer.button(now(), button, ButtonState::Pressed);
    pointer.frame();
    queue.roundtrip(&mut state).unwrap_or_else(|error| fail(error));
    std::thread::sleep(Duration::from_millis(80));

    pointer.button(now(), button, ButtonState::Released);
    pointer.frame();
    queue.roundtrip(&mut state).unwrap_or_else(|error| fail(error));
    pointer.destroy();
    queue.roundtrip(&mut state).unwrap_or_else(|error| fail(error));
}
