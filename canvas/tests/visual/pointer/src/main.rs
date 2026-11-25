//! Minimal deterministic pointer for the visual regression suite.
//!
//! Usage:
//!   wlr-virtual-pointer click  <x> <y> <left|right> --extent <w>x<h> --output <name>
//!   wlr-virtual-pointer scroll <x> <y> <up|down>    --extent <w>x<h> --output <name>
//!   wlr-virtual-pointer drag   <x1> <y1> <x2> <y2>  --extent <w>x<h> --output <name>
//!
//! It attaches a virtual pointer to the existing wlroots seat and emits
//! absolute motion plus press/release or axis events on the named output.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_output::{self, WlOutput},
        wl_pointer::{Axis, ButtonState},
        wl_registry::{self, WlRegistry},
        wl_seat::WlSeat,
    },
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

/// Every bound output and the compositor-assigned name, when received.
#[derive(Default)]
struct State {
    outputs: Vec<(WlOutput, Option<String>)>,
}

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

impl Dispatch<WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        output: &WlOutput,
        event: <WlOutput as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            for (candidate, candidate_name) in &mut state.outputs {
                if candidate.id() == output.id() {
                    *candidate_name = Some(name.clone());
                }
            }
        }
    }
}

delegate_noop!(State: ignore WlSeat);
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

fn motion(
    pointer: &ZwlrVirtualPointerV1,
    queue: &mut wayland_client::EventQueue<State>,
    state: &mut State,
    x: u32,
    y: u32,
    extent: (u32, u32),
) {
    pointer.motion_absolute(now(), x, y, extent.0, extent.1);
    pointer.frame();
    queue.roundtrip(state).unwrap_or_else(|error| fail(error));
    std::thread::sleep(Duration::from_millis(60));
}

fn button(pointer: &ZwlrVirtualPointerV1, code: u32, pressed: bool) {
    pointer.button(
        now(),
        code,
        if pressed { ButtonState::Pressed } else { ButtonState::Released },
    );
    pointer.frame();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or_default();
    let (extent_at, output_at) = match command {
        "click" | "scroll" => (4, 6),
        "drag" => (5, 7),
        _ => fail("unknown command"),
    };
    if args.len() != output_at + 2
        || args[extent_at] != "--extent"
        || args[output_at] != "--output"
    {
        fail("usage: click|scroll|drag ... --extent <w>x<h> --output <name>");
    }
    let extent = args[extent_at + 1]
        .split_once('x')
        .unwrap_or_else(|| fail("extent must be WIDTHxHEIGHT"));
    let extent_x: u32 = extent.0.parse().unwrap_or_else(|_| fail("bad extent width"));
    let extent_y: u32 = extent.1.parse().unwrap_or_else(|_| fail("bad extent height"));
    let wanted_output = &args[output_at + 1];

    let connection = Connection::connect_to_env().unwrap_or_else(|error| fail(error));
    let (globals, mut queue) =
        registry_queue_init::<State>(&connection).unwrap_or_else(|error| fail(error));
    let qh = queue.handle();
    let mut state = State::default();

    for global in globals.contents().clone_list() {
        if global.interface != "wl_output" {
            continue;
        }
        let output = globals.registry().bind::<WlOutput, (), State>(
            global.name,
            global.version.min(4),
            &qh,
            (),
        );
        state.outputs.push((output, None));
    }
    queue.roundtrip(&mut state).unwrap_or_else(|error| fail(error));

    let output = state
        .outputs
        .iter()
        .find(|(_, name)| name.as_deref() == Some(wanted_output.as_str()))
        .map(|(output, _)| output.clone())
        .unwrap_or_else(|| {
            let names: Vec<&str> = state.outputs.iter().filter_map(|(_, n)| n.as_deref()).collect();
            fail(format!("no output named {wanted_output}; available: {}", names.join(", ")))
        });

    let seat = globals
        .bind::<WlSeat, _, _>(&qh, 1..=7, ())
        .unwrap_or_else(|error| fail(format!("no wl_seat: {error}")));
    let manager = globals
        .bind::<ZwlrVirtualPointerManagerV1, _, _>(&qh, 1..=2, ())
        .unwrap_or_else(|error| fail(format!("no virtual pointer manager: {error}")));
    let pointer = manager.create_virtual_pointer_with_output(Some(&seat), Some(&output), &qh, ());

    match command {
        "click" => {
            let x: u32 = args[1].parse().unwrap_or_else(|_| fail("x must be an integer"));
            let y: u32 = args[2].parse().unwrap_or_else(|_| fail("y must be an integer"));
            let code = match args[3].as_str() {
                "left" => 0x110,
                "right" => 0x111,
                other => fail(format!("unknown button {other}")),
            };
            motion(&pointer, &mut queue, &mut state, x, y, (extent_x, extent_y));
            button(&pointer, code, true);
            queue.roundtrip(&mut state).unwrap_or_else(|error| fail(error));
            std::thread::sleep(Duration::from_millis(80));
            button(&pointer, code, false);
        }
        "scroll" => {
            let x: u32 = args[1].parse().unwrap_or_else(|_| fail("x must be an integer"));
            let y: u32 = args[2].parse().unwrap_or_else(|_| fail("y must be an integer"));
            let value = match args[3].as_str() {
                "up" => 120.0,
                "down" => -120.0,
                other => fail(format!("unknown direction {other}")),
            };
            motion(&pointer, &mut queue, &mut state, x, y, (extent_x, extent_y));
            for _ in 0..3 {
                pointer.axis(now(), Axis::VerticalScroll, value);
                pointer.frame();
                queue.roundtrip(&mut state).unwrap_or_else(|error| fail(error));
                std::thread::sleep(Duration::from_millis(80));
            }
        }
        "drag" => {
            let x1: u32 = args[1].parse().unwrap_or_else(|_| fail("x1 must be an integer"));
            let y1: u32 = args[2].parse().unwrap_or_else(|_| fail("y1 must be an integer"));
            let x2: u32 = args[3].parse().unwrap_or_else(|_| fail("x2 must be an integer"));
            let y2: u32 = args[4].parse().unwrap_or_else(|_| fail("y2 must be an integer"));
            motion(&pointer, &mut queue, &mut state, x1, y1, (extent_x, extent_y));
            button(&pointer, 0x110, true);
            queue.roundtrip(&mut state).unwrap_or_else(|error| fail(error));
            std::thread::sleep(Duration::from_millis(80));
            motion(&pointer, &mut queue, &mut state, x2, y2, (extent_x, extent_y));
            button(&pointer, 0x110, false);
        }
        _ => unreachable!(),
    }
    queue.roundtrip(&mut state).unwrap_or_else(|error| fail(error));
    pointer.destroy();
    queue.roundtrip(&mut state).unwrap_or_else(|error| fail(error));
}
