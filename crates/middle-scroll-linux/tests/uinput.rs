//! End-to-end checks of the virtual mouse against the kernel's uinput.
//!
//! They need `/dev/uinput` and read access to `/dev/input`, so they are
//! ignored by default. Run them as root:
//!
//! ```text
//! sudo -E cargo test -p wayland-wheeltani --test uinput -- --ignored
//! ```
//!
//! Each test grabs the virtual mouse it creates, so the clicks and scrolls
//! never reach the desktop.
#![cfg(target_os = "linux")]
// Each fixture holds the serial lock for its whole test on purpose.
#![allow(clippy::significant_drop_tightening)]

use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use evdev::{Device, EventType, InputEvent, KeyCode, RelativeAxisCode};
use middle_scroll_core::{CoreAction, MouseButton};
use wayland_wheeltani::device_discovery;
use wayland_wheeltani::virtual_mouse::{VirtualMouse, BASE_AXES, BASE_KEYS, VIRTUAL_MOUSE_NAME};

/// How long udev and the kernel get to deliver a node or an event.
const WAIT: Duration = Duration::from_secs(3);

/// The tests find their device by name, so they must not overlap.
static SERIAL: Mutex<()> = Mutex::new(());

struct Fixture {
    mouse: VirtualMouse,
    reader: Device,
    path: PathBuf,
    _serial: MutexGuard<'static, ()>,
}

fn virtual_nodes() -> HashSet<PathBuf> {
    evdev::enumerate()
        .filter(|(_, device)| device.name() == Some(VIRTUAL_MOUSE_NAME))
        .map(|(path, _)| path)
        .collect()
}

/// Creates a virtual mouse, then opens and grabs its event node once udev
/// has created it. Nodes that existed before (a running daemon) are skipped.
fn fixture() -> Fixture {
    let serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let before = virtual_nodes();
    let mouse = VirtualMouse::new(KeyCode::BTN_MIDDLE, &[], &[]).expect("create the uinput device");
    let deadline = Instant::now() + WAIT;
    let path = loop {
        if let Some(path) = virtual_nodes().difference(&before).next() {
            break path.clone();
        }
        assert!(
            Instant::now() < deadline,
            "the virtual mouse node never appeared"
        );
        thread::sleep(Duration::from_millis(20));
    };
    let mut reader = Device::open(&path).expect("open the virtual mouse node");
    reader.grab().expect("grab the virtual mouse");
    reader.set_nonblocking(true).expect("non-blocking reads");
    Fixture {
        mouse,
        reader,
        path,
        _serial: serial,
    }
}

/// Reads events, without the `SYN_REPORT`s, until `count` have arrived or
/// time runs out.
fn read_events(reader: &mut Device, count: usize) -> Vec<InputEvent> {
    let mut events = Vec::new();
    let deadline = Instant::now() + WAIT;
    while events.len() < count && Instant::now() < deadline {
        match reader.fetch_events() {
            Ok(batch) => {
                events.extend(batch.filter(|e| e.event_type() != EventType::SYNCHRONIZATION));
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(err) => panic!("reading the virtual mouse failed: {err}"),
        }
    }
    events
}

fn key(code: KeyCode, pressed: bool) -> (EventType, u16, i32) {
    (EventType::KEY, code.0, i32::from(pressed))
}

const fn rel(code: RelativeAxisCode, value: i32) -> (EventType, u16, i32) {
    (EventType::RELATIVE, code.0, value)
}

fn simplify(events: &[InputEvent]) -> Vec<(EventType, u16, i32)> {
    events
        .iter()
        .map(|e| (e.event_type(), e.code(), e.value()))
        .collect()
}

#[test]
#[ignore = "needs /dev/uinput and root"]
fn virtual_mouse_has_its_buttons_and_axes_and_is_never_offered() {
    let fixture = fixture();
    let keys = fixture.reader.supported_keys().expect("buttons");
    for code in BASE_KEYS {
        assert!(keys.contains(code), "missing {code:?}");
    }
    let axes = fixture.reader.supported_relative_axes().expect("axes");
    for code in BASE_AXES {
        assert!(axes.contains(code), "missing {code:?}");
    }
    // --setup and device matching must not pick the daemon's own output.
    assert!(device_discovery::enumerate_mice()
        .iter()
        .all(|mouse| mouse.path != fixture.path));
    assert!(device_discovery::probe(&fixture.path).is_none());
}

#[test]
#[ignore = "needs /dev/uinput and root"]
fn a_wheel_detent_carries_its_hi_res_value() {
    let mut fixture = fixture();
    fixture
        .mouse
        .push_action(&CoreAction::ForwardWheel {
            vertical: -1,
            horizontal: 0,
        })
        .unwrap();
    fixture.mouse.flush().unwrap();
    let events = read_events(&mut fixture.reader, 2);
    assert_eq!(
        simplify(&events),
        [
            rel(RelativeAxisCode::REL_WHEEL, -1),
            rel(RelativeAxisCode::REL_WHEEL_HI_RES, -120),
        ]
    );
}

#[test]
#[ignore = "needs /dev/uinput and root"]
fn hi_res_scrolling_sends_only_hi_res_events() {
    let mut fixture = fixture();
    fixture
        .mouse
        .push_action(&CoreAction::EmitWheelHiRes {
            vertical_units: 30,
            horizontal_units: -15,
        })
        .unwrap();
    fixture.mouse.flush().unwrap();
    let events = read_events(&mut fixture.reader, 2);
    assert_eq!(
        simplify(&events),
        [
            rel(RelativeAxisCode::REL_WHEEL_HI_RES, 30),
            rel(RelativeAxisCode::REL_HWHEEL_HI_RES, -15),
        ]
    );
}

#[test]
#[ignore = "needs /dev/uinput and root"]
fn middle_click_is_a_press_then_a_release() {
    let mut fixture = fixture();
    fixture
        .mouse
        .push_action(&CoreAction::EmitMiddleClick)
        .unwrap();
    let events = read_events(&mut fixture.reader, 2);
    assert_eq!(
        simplify(&events),
        [
            key(KeyCode::BTN_MIDDLE, true),
            key(KeyCode::BTN_MIDDLE, false),
        ]
    );
}

#[test]
#[ignore = "needs /dev/uinput and root"]
fn buttons_and_motion_are_forwarded() {
    let mut fixture = fixture();
    let mouse = &mut fixture.mouse;
    mouse
        .push_action(&CoreAction::ForwardMouseButton {
            button: MouseButton::Left,
            pressed: true,
        })
        .unwrap();
    mouse
        .push_action(&CoreAction::ForwardMotion { dx: 5, dy: -3 })
        .unwrap();
    mouse.flush().unwrap();
    mouse
        .push_action(&CoreAction::ForwardMouseButton {
            button: MouseButton::Left,
            pressed: false,
        })
        .unwrap();
    mouse.flush().unwrap();
    let events = read_events(&mut fixture.reader, 4);
    assert_eq!(
        simplify(&events),
        [
            key(KeyCode::BTN_LEFT, true),
            rel(RelativeAxisCode::REL_X, 5),
            rel(RelativeAxisCode::REL_Y, -3),
            key(KeyCode::BTN_LEFT, false),
        ]
    );
}

#[test]
#[ignore = "needs /dev/uinput and root"]
fn release_all_buttons_unsticks_a_held_button() {
    let mut fixture = fixture();
    fixture
        .mouse
        .push_action(&CoreAction::ForwardMouseButton {
            button: MouseButton::Right,
            pressed: true,
        })
        .unwrap();
    fixture.mouse.flush().unwrap();
    fixture.mouse.release_all_buttons().unwrap();
    // The kernel drops releases of buttons that are not down, so only the
    // right button's release comes through.
    let events = read_events(&mut fixture.reader, 2);
    assert_eq!(
        simplify(&events),
        [
            key(KeyCode::BTN_RIGHT, true),
            key(KeyCode::BTN_RIGHT, false),
        ]
    );
}
