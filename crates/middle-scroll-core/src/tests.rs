use crate::config::{CoreConfig, SpeedStep};
use crate::engine::Engine;
use crate::model::{CoreAction, CoreInputEvent, EngineState, MouseButton};

fn engine() -> Engine {
    Engine::new(CoreConfig::default())
}

fn engine_with(cfg: CoreConfig) -> Engine {
    Engine::new(cfg)
}

fn count_detents(actions: &[CoreAction]) -> i32 {
    actions
        .iter()
        .filter_map(|a| match a {
            CoreAction::EmitWheelDetents { vertical, .. } => Some(*vertical),
            _ => None,
        })
        .sum()
}

fn count_hires(actions: &[CoreAction]) -> i32 {
    actions
        .iter()
        .filter_map(|a| match a {
            CoreAction::EmitWheelHiRes { vertical_units, .. } => Some(*vertical_units),
            _ => None,
        })
        .sum()
}

fn count_hires_events(actions: &[CoreAction]) -> usize {
    actions
        .iter()
        .filter(|a| matches!(a, CoreAction::EmitWheelHiRes { .. }))
        .count()
}

fn count_detents_h(actions: &[CoreAction]) -> i32 {
    actions
        .iter()
        .filter_map(|a| match a {
            CoreAction::EmitWheelDetents { horizontal, .. } => Some(*horizontal),
            _ => None,
        })
        .sum()
}

fn count_hires_h(actions: &[CoreAction]) -> i32 {
    actions
        .iter()
        .filter_map(|a| match a {
            CoreAction::EmitWheelHiRes {
                horizontal_units, ..
            } => Some(*horizontal_units),
            _ => None,
        })
        .sum()
}

fn run_for(engine: &mut Engine, secs: f64, dt_micros: u64) -> Vec<CoreAction> {
    let total_us = (secs * 1_000_000.0) as u64;
    let mut all = Vec::new();
    let mut elapsed = 0u64;
    while elapsed < total_us {
        all.extend(engine.process(CoreInputEvent::Tick { dt_micros }));
        elapsed += dt_micros;
    }
    all
}

#[test]
fn config_defaults_validate() {
    CoreConfig::default().validate().unwrap();
}

#[test]
fn config_rejects_negative_deadzone() {
    let cfg = CoreConfig {
        deadzone_units: -1,
        ..CoreConfig::default()
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn config_rejects_max_offset_smaller_than_deadzone() {
    let cfg = CoreConfig {
        deadzone_units: 100,
        max_offset_units: 50,
        ..CoreConfig::default()
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn config_rejects_max_speed_below_min_speed() {
    let cfg = CoreConfig {
        min_speed_detents_per_second: 10.0,
        max_speed_detents_per_second: 5.0,
        ..CoreConfig::default()
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn config_rejects_speed_step_inside_deadzone() {
    let cfg = CoreConfig {
        scroll_speed_steps: vec![SpeedStep {
            distance_units: 10,
            speed_detents_per_second: 2.0,
        }],
        ..CoreConfig::default()
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn config_rejects_unsorted_speed_steps() {
    let cfg = CoreConfig {
        scroll_speed_steps: vec![
            SpeedStep {
                distance_units: 80,
                speed_detents_per_second: 8.0,
            },
            SpeedStep {
                distance_units: 40,
                speed_detents_per_second: 4.0,
            },
        ],
        ..CoreConfig::default()
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn config_rejects_bad_speed_step_speed() {
    let cfg = CoreConfig {
        scroll_speed_steps: vec![SpeedStep {
            distance_units: 40,
            speed_detents_per_second: f64::INFINITY,
        }],
        ..CoreConfig::default()
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn config_rejects_bad_min_hires_units_per_event() {
    for min_hires_units_per_event in [0, 121] {
        let cfg = CoreConfig {
            min_hires_units_per_event,
            ..CoreConfig::default()
        };
        assert!(cfg.validate().is_err());
    }
}

#[test]
fn click_court_emits_middle_click_and_returns_to_idle() {
    let mut e = engine();
    let down = e.process(CoreInputEvent::MiddleDown);
    assert_eq!(down, vec![CoreAction::Suppress]);
    assert_eq!(e.state(), EngineState::MiddlePending);

    let up = e.process(CoreInputEvent::MiddleUp);
    assert_eq!(up, vec![CoreAction::EmitMiddleClick]);
    assert_eq!(e.state(), EngineState::Idle);
}

#[test]
fn deadzone_keeps_engine_pending_and_does_not_scroll() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    let actions = e.process(CoreInputEvent::Motion { dx: 0, dy: 5 });

    assert!(actions
        .iter()
        .all(|a| !matches!(a, CoreAction::EnterScrollMode)));
    assert_eq!(e.state(), EngineState::MiddlePending);

    let tick = e.process(CoreInputEvent::Tick { dt_micros: 8333 });
    assert!(tick.is_empty());
}

#[test]
fn motion_above_deadzone_enters_scroll_mode() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    let actions = e.process(CoreInputEvent::Motion { dx: 0, dy: 11 });
    assert!(actions.contains(&CoreAction::EnterScrollMode));
    assert_eq!(e.state(), EngineState::Scrolling);
}

#[test]
fn click_after_motion_inside_deadzone_still_emits_middle_click() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 1, dy: 3 });
    e.process(CoreInputEvent::Motion { dx: -1, dy: -2 });
    let up = e.process(CoreInputEvent::MiddleUp);
    assert_eq!(up, vec![CoreAction::EmitMiddleClick]);
    assert_eq!(e.state(), EngineState::Idle);
}

#[test]
fn scroll_down_emits_negative_legacy_detents_after_one_second() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 120 });
    let actions = run_for(&mut e, 1.0, 8333);
    let total = count_detents(&actions);
    assert!(total < 0, "expected negative detents, got {total}");
}

#[test]
fn scroll_up_emits_positive_legacy_detents_after_one_second() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: -120 });
    let actions = run_for(&mut e, 1.0, 8333);
    let total = count_detents(&actions);
    assert!(total > 0, "expected positive detents, got {total}");
}

#[test]
fn higher_offset_produces_more_detents_per_second() {
    let mut slow = engine();
    slow.process(CoreInputEvent::MiddleDown);
    slow.process(CoreInputEvent::Motion { dx: 0, dy: 30 });
    let slow_actions = run_for(&mut slow, 1.0, 8333);

    let mut fast = engine();
    fast.process(CoreInputEvent::MiddleDown);
    fast.process(CoreInputEvent::Motion { dx: 0, dy: 120 });
    let fast_actions = run_for(&mut fast, 1.0, 8333);

    let slow_total = count_detents(&slow_actions).abs();
    let fast_total = count_detents(&fast_actions).abs();
    assert!(
        fast_total > slow_total,
        "expected fast ({fast_total}) > slow ({slow_total})"
    );
}

#[test]
fn return_to_deadzone_stops_scrolling() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 120 });
    e.process(CoreInputEvent::Motion { dx: 0, dy: -115 });

    assert!(e.offset_y_units().abs() <= 10);

    let mut total = 0i32;
    for _ in 0..120 {
        let actions = e.process(CoreInputEvent::Tick { dt_micros: 8333 });
        total += count_detents(&actions);
    }
    assert_eq!(total, 0, "no detents should be emitted in deadzone");
}

#[test]
fn crossing_zero_inverts_scroll_direction() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 100 });
    let down_actions = run_for(&mut e, 0.5, 8333);
    let down_total = count_detents(&down_actions);
    assert!(down_total < 0, "expected scroll down, got {down_total}");

    e.process(CoreInputEvent::Motion { dx: 0, dy: -220 });
    assert!(e.offset_y_units() < 0);

    let up_actions = run_for(&mut e, 0.5, 8333);
    let up_total = count_detents(&up_actions);
    assert!(
        up_total > 0,
        "expected scroll up after inversion, got {up_total}"
    );
}

#[test]
fn middle_release_during_scroll_returns_to_idle_without_extra_click() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 100 });
    let _ = run_for(&mut e, 0.2, 8333);

    let release = e.process(CoreInputEvent::MiddleUp);
    assert!(release.contains(&CoreAction::ExitScrollMode));
    assert!(!release.contains(&CoreAction::EmitMiddleClick));
    assert_eq!(e.state(), EngineState::Idle);

    let post = e.process(CoreInputEvent::Tick { dt_micros: 8333 });
    assert!(post.is_empty(), "no scroll should be emitted after release");
}

#[test]
fn invert_vertical_flips_scroll_signs() {
    let cfg = CoreConfig {
        invert_vertical: true,
        ..CoreConfig::default()
    };
    let mut e = engine_with(cfg);

    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 120 });
    let actions = run_for(&mut e, 1.0, 8333);
    let total = count_detents(&actions);
    assert!(
        total > 0,
        "with invert, mouse-down should scroll up; got {total}"
    );
}

#[test]
fn hires_emission_disabled_via_config() {
    let cfg = CoreConfig {
        emit_hires_wheel: false,
        emit_legacy_wheel: true,
        ..CoreConfig::default()
    };
    let mut e = engine_with(cfg);

    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 120 });
    let actions = run_for(&mut e, 1.0, 8333);
    assert_eq!(count_hires(&actions), 0);
    assert!(count_detents(&actions).abs() > 0);
}

#[test]
fn legacy_emission_disabled_via_config() {
    let cfg = CoreConfig {
        emit_hires_wheel: true,
        emit_legacy_wheel: false,
        ..CoreConfig::default()
    };
    let mut e = engine_with(cfg);

    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 120 });
    let actions = run_for(&mut e, 1.0, 8333);
    assert_eq!(count_detents(&actions), 0);
    assert!(count_hires(&actions).abs() > 0);
}

#[test]
fn hires_units_track_legacy_detents_at_120_per_detent() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 240 });
    let actions = run_for(&mut e, 2.0, 8333);

    let detents = count_detents(&actions);
    let hires = count_hires(&actions);

    let ratio = f64::from(hires) / f64::from(detents);
    assert!(
        (ratio - 120.0).abs() < 5.0,
        "expected ~120 hi-res units per detent, got ratio={ratio}"
    );
}

#[test]
fn hires_events_are_grouped_by_minimum_units_without_changing_total_distance() {
    let cfg = CoreConfig {
        emit_legacy_wheel: false,
        min_hires_units_per_event: 15,
        scroll_speed_steps: vec![SpeedStep {
            distance_units: 11,
            speed_detents_per_second: 1.5,
        }],
        ..CoreConfig::default()
    };
    let mut grouped = engine_with(cfg.clone());
    grouped.process(CoreInputEvent::MiddleDown);
    grouped.process(CoreInputEvent::Motion { dx: 0, dy: 11 });
    let grouped_actions = run_for(&mut grouped, 1.0, 8333);

    let mut ungrouped = engine_with(CoreConfig {
        min_hires_units_per_event: 1,
        ..cfg
    });
    ungrouped.process(CoreInputEvent::MiddleDown);
    ungrouped.process(CoreInputEvent::Motion { dx: 0, dy: 11 });
    let ungrouped_actions = run_for(&mut ungrouped, 1.0, 8333);

    assert!(
        count_hires_events(&grouped_actions) < count_hires_events(&ungrouped_actions),
        "grouping should reduce hi-res event count"
    );

    let grouped_units = count_hires(&grouped_actions).abs();
    let ungrouped_units = count_hires(&ungrouped_actions).abs();
    assert!(
        (ungrouped_units - grouped_units).abs() <= 15,
        "grouping should preserve total distance within one threshold: grouped={grouped_units}, ungrouped={ungrouped_units}"
    );
}

#[test]
fn motion_is_suppressed_during_pending_when_configured() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    let actions = e.process(CoreInputEvent::Motion { dx: 5, dy: 3 });
    assert!(actions.contains(&CoreAction::Suppress));
    assert!(!actions
        .iter()
        .any(|a| matches!(a, CoreAction::ForwardMotion { .. })));
}

#[test]
fn motion_is_suppressed_during_scrolling_when_configured() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 50 });
    let actions = e.process(CoreInputEvent::Motion { dx: 100, dy: 1 });
    assert!(actions.contains(&CoreAction::Suppress));
    assert!(!actions
        .iter()
        .any(|a| matches!(a, CoreAction::ForwardMotion { .. })));
}

#[test]
fn motion_is_forwarded_when_suppress_disabled() {
    let cfg = CoreConfig {
        suppress_motion_while_pending: false,
        suppress_motion_while_scrolling: false,
        ..CoreConfig::default()
    };
    let mut e = engine_with(cfg);

    e.process(CoreInputEvent::MiddleDown);
    let pending = e.process(CoreInputEvent::Motion { dx: 4, dy: 3 });
    assert!(pending.contains(&CoreAction::ForwardMotion { dx: 4, dy: 3 }));

    e.process(CoreInputEvent::Motion { dx: 0, dy: 50 });
    let scrolling = e.process(CoreInputEvent::Motion { dx: 7, dy: 1 });
    assert!(scrolling.contains(&CoreAction::ForwardMotion { dx: 7, dy: 1 }));
}

#[test]
fn replay_pending_motion_replays_on_short_click() {
    let cfg = CoreConfig {
        replay_pending_motion_on_click: true,
        ..CoreConfig::default()
    };
    let mut e = engine_with(cfg);

    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 2, dy: 1 });
    e.process(CoreInputEvent::Motion { dx: -1, dy: 2 });
    let release = e.process(CoreInputEvent::MiddleUp);

    let forwarded: Vec<_> = release
        .iter()
        .filter_map(|a| match a {
            CoreAction::ForwardMotion { dx, dy } => Some((*dx, *dy)),
            _ => None,
        })
        .collect();
    assert_eq!(forwarded, vec![(2, 1), (-1, 2)]);
    assert!(release.contains(&CoreAction::EmitMiddleClick));
}

#[test]
fn left_button_passthrough_in_idle() {
    let mut e = engine();
    let down = e.process(CoreInputEvent::LeftDown);
    assert_eq!(
        down,
        vec![CoreAction::ForwardMouseButton {
            button: MouseButton::Left,
            pressed: true,
        }]
    );
    let up = e.process(CoreInputEvent::LeftUp);
    assert_eq!(
        up,
        vec![CoreAction::ForwardMouseButton {
            button: MouseButton::Left,
            pressed: false,
        }]
    );
}

#[test]
fn right_button_passthrough_during_scrolling() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 50 });
    let click = e.process(CoreInputEvent::RightDown);
    assert_eq!(
        click,
        vec![CoreAction::ForwardMouseButton {
            button: MouseButton::Right,
            pressed: true,
        }]
    );
}

#[test]
fn real_wheel_passes_through_in_all_states() {
    let mut e = engine();
    let idle = e.process(CoreInputEvent::Wheel {
        vertical: 1,
        horizontal: 0,
    });
    assert!(matches!(
        idle.as_slice(),
        [CoreAction::ForwardWheel {
            vertical: 1,
            horizontal: 0
        }]
    ));

    e.process(CoreInputEvent::MiddleDown);
    let pending = e.process(CoreInputEvent::Wheel {
        vertical: -1,
        horizontal: 0,
    });
    assert!(matches!(
        pending.as_slice(),
        [CoreAction::ForwardWheel {
            vertical: -1,
            horizontal: 0
        }]
    ));

    e.process(CoreInputEvent::Motion { dx: 0, dy: 50 });
    let scrolling = e.process(CoreInputEvent::Wheel {
        vertical: 2,
        horizontal: 0,
    });
    assert!(matches!(
        scrolling.as_slice(),
        [CoreAction::ForwardWheel {
            vertical: 2,
            horizontal: 0
        }]
    ));
}

#[test]
fn offset_is_clamped_to_max_offset_units() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 100_000 });
    assert_eq!(e.offset_y_units(), 240);

    e.process(CoreInputEvent::Motion {
        dx: 0,
        dy: -1_000_000,
    });
    assert_eq!(e.offset_y_units(), -240);
}

#[test]
fn detents_are_capped_per_tick() {
    let cfg = CoreConfig {
        max_detents_per_tick: 2,
        max_speed_detents_per_second: 1_000_000.0,
        scroll_speed_steps: vec![SpeedStep {
            distance_units: 11,
            speed_detents_per_second: 1_000_000.0,
        }],
        ..CoreConfig::default()
    };
    let mut e = engine_with(cfg);

    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 240 });
    let actions = e.process(CoreInputEvent::Tick {
        dt_micros: 1_000_000,
    });

    for action in &actions {
        if let CoreAction::EmitWheelDetents { vertical, .. } = action {
            assert!(vertical.abs() <= 2, "got {vertical} in single emit");
        }
    }
}

#[test]
fn scroll_speed_steps_pick_last_reached_distance() {
    let cfg = CoreConfig {
        emit_hires_wheel: false,
        scroll_speed_steps: vec![
            SpeedStep {
                distance_units: 20,
                speed_detents_per_second: 2.0,
            },
            SpeedStep {
                distance_units: 100,
                speed_detents_per_second: 8.0,
            },
        ],
        ..CoreConfig::default()
    };

    let mut slow = engine_with(cfg.clone());
    slow.process(CoreInputEvent::MiddleDown);
    slow.process(CoreInputEvent::Motion { dx: 0, dy: 30 });
    let slow_detents = count_detents(&run_for(&mut slow, 1.0, 100_000)).abs();

    let mut fast = engine_with(cfg);
    fast.process(CoreInputEvent::MiddleDown);
    fast.process(CoreInputEvent::Motion { dx: 0, dy: 120 });
    let fast_detents = count_detents(&run_for(&mut fast, 1.0, 100_000)).abs();

    assert!(
        (1..=3).contains(&slow_detents),
        "expected roughly 2 detents, got {slow_detents}"
    );
    assert!(
        (7..=9).contains(&fast_detents),
        "expected roughly 8 detents, got {fast_detents}"
    );
    assert!(fast_detents > slow_detents);
}

#[test]
fn empty_scroll_speed_steps_fall_back_to_continuous_curve() {
    let cfg = CoreConfig {
        emit_hires_wheel: false,
        scroll_speed_steps: Vec::new(),
        min_speed_detents_per_second: 5.0,
        max_speed_detents_per_second: 5.0,
        ..CoreConfig::default()
    };
    let mut e = engine_with(cfg);

    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 40 });
    let detents = count_detents(&run_for(&mut e, 1.0, 100_000)).abs();

    assert!(
        (4..=6).contains(&detents),
        "expected roughly 5 detents, got {detents}"
    );
}

#[test]
fn zero_dt_tick_emits_nothing() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 120 });
    let actions = e.process(CoreInputEvent::Tick { dt_micros: 0 });
    assert!(actions.is_empty());
}

#[test]
fn middle_down_resets_offset_and_accumulators_from_previous_session() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 200 });
    let _ = run_for(&mut e, 1.0, 8333);
    e.process(CoreInputEvent::MiddleUp);

    assert_eq!(e.offset_y_units(), 0);
    e.process(CoreInputEvent::MiddleDown);
    assert_eq!(e.offset_y_units(), 0);
    let nothing = e.process(CoreInputEvent::Tick { dt_micros: 8333 });
    assert!(nothing.is_empty());
}

#[test]
fn speed_curve_respects_min_at_just_above_deadzone() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 11 });

    let actions = run_for(&mut e, 1.0, 8333);
    let detents = count_detents(&actions).abs();
    let cfg = CoreConfig::default();
    let expected_min = cfg.min_speed_detents_per_second.floor() as i32;
    assert!(
        detents >= expected_min,
        "expected at least {expected_min} detents at min speed, got {detents}"
    );
}

#[test]
fn speed_curve_respects_max_at_full_speed_offset() {
    let mut e = engine();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 0, dy: 240 });

    let actions = run_for(&mut e, 1.0, 8333);
    let detents = count_detents(&actions).abs();
    let cfg = CoreConfig::default();
    let max = cfg.max_speed_detents_per_second.ceil() as i32;
    let cap = cfg.tick_hz as i32 * cfg.max_detents_per_tick;
    let upper = max.min(cap) + 2;
    assert!(
        detents <= upper,
        "expected <= {upper} detents at max speed, got {detents}"
    );
}

fn engine_horizontal() -> Engine {
    engine_with(CoreConfig {
        horizontal_scroll: true,
        ..CoreConfig::default()
    })
}

#[test]
fn horizontal_motion_is_ignored_when_horizontal_scroll_disabled() {
    let mut e = engine_with(CoreConfig {
        horizontal_scroll: false,
        ..CoreConfig::default()
    });
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 200, dy: 0 });

    assert_eq!(e.offset_x_units(), 0);
    assert_eq!(e.state(), EngineState::MiddlePending);

    let actions = run_for(&mut e, 1.0, 8333);
    assert_eq!(count_detents_h(&actions), 0);
    assert_eq!(count_hires_h(&actions), 0);
}

#[test]
fn horizontal_motion_above_deadzone_enters_scroll_mode() {
    let mut e = engine_horizontal();
    e.process(CoreInputEvent::MiddleDown);
    let actions = e.process(CoreInputEvent::Motion { dx: 11, dy: 0 });
    assert!(actions.contains(&CoreAction::EnterScrollMode));
    assert_eq!(e.state(), EngineState::Scrolling);
}

#[test]
fn horizontal_deadzone_does_not_scroll_inside_band() {
    let mut e = engine_horizontal();
    e.process(CoreInputEvent::MiddleDown);
    let actions = e.process(CoreInputEvent::Motion { dx: 5, dy: 0 });

    assert!(!actions.contains(&CoreAction::EnterScrollMode));
    assert_eq!(e.state(), EngineState::MiddlePending);

    let tick = e.process(CoreInputEvent::Tick { dt_micros: 8333 });
    assert!(tick.is_empty());
}

#[test]
fn scroll_right_emits_positive_horizontal_legacy_detents() {
    let mut e = engine_horizontal();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 120, dy: 0 });
    let actions = run_for(&mut e, 1.0, 8333);
    let total = count_detents_h(&actions);
    assert!(
        total > 0,
        "expected positive horizontal detents, got {total}"
    );
    assert_eq!(
        count_detents(&actions),
        0,
        "no vertical detents expected for pure horizontal motion"
    );
}

#[test]
fn scroll_left_emits_negative_horizontal_legacy_detents() {
    let mut e = engine_horizontal();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: -120, dy: 0 });
    let actions = run_for(&mut e, 1.0, 8333);
    let total = count_detents_h(&actions);
    assert!(
        total < 0,
        "expected negative horizontal detents, got {total}"
    );
}

#[test]
fn higher_horizontal_offset_produces_more_detents_per_second() {
    let mut slow = engine_horizontal();
    slow.process(CoreInputEvent::MiddleDown);
    slow.process(CoreInputEvent::Motion { dx: 30, dy: 0 });
    let slow_actions = run_for(&mut slow, 1.0, 8333);

    let mut fast = engine_horizontal();
    fast.process(CoreInputEvent::MiddleDown);
    fast.process(CoreInputEvent::Motion { dx: 120, dy: 0 });
    let fast_actions = run_for(&mut fast, 1.0, 8333);

    let slow_total = count_detents_h(&slow_actions).abs();
    let fast_total = count_detents_h(&fast_actions).abs();
    assert!(
        fast_total > slow_total,
        "expected fast ({fast_total}) > slow ({slow_total})"
    );
}

#[test]
fn horizontal_return_to_deadzone_stops_scrolling() {
    let mut e = engine_horizontal();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 120, dy: 0 });
    e.process(CoreInputEvent::Motion { dx: -115, dy: 0 });

    assert!(e.offset_x_units().abs() <= 10);

    let mut total = 0i32;
    for _ in 0..120 {
        let actions = e.process(CoreInputEvent::Tick { dt_micros: 8333 });
        total += count_detents_h(&actions);
    }
    assert_eq!(
        total, 0,
        "no horizontal detents should be emitted in deadzone"
    );
}

#[test]
fn horizontal_crossing_zero_inverts_scroll_direction() {
    let mut e = engine_horizontal();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 100, dy: 0 });
    let right = run_for(&mut e, 0.5, 8333);
    let right_total = count_detents_h(&right);
    assert!(right_total > 0, "expected scroll right, got {right_total}");

    e.process(CoreInputEvent::Motion { dx: -220, dy: 0 });
    assert!(e.offset_x_units() < 0);

    let left = run_for(&mut e, 0.5, 8333);
    let left_total = count_detents_h(&left);
    assert!(
        left_total < 0,
        "expected scroll left after inversion, got {left_total}"
    );
}

#[test]
fn invert_horizontal_flips_scroll_signs() {
    let cfg = CoreConfig {
        horizontal_scroll: true,
        invert_horizontal: true,
        ..CoreConfig::default()
    };
    let mut e = engine_with(cfg);

    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 120, dy: 0 });
    let actions = run_for(&mut e, 1.0, 8333);
    let total = count_detents_h(&actions);
    assert!(
        total < 0,
        "with invert_horizontal, mouse-right should produce negative detents; got {total}"
    );
}

#[test]
fn horizontal_hires_units_track_legacy_detents_at_120_per_detent() {
    let mut e = engine_horizontal();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 240, dy: 0 });
    let actions = run_for(&mut e, 2.0, 8333);

    let detents = count_detents_h(&actions);
    let hires = count_hires_h(&actions);

    let ratio = f64::from(hires) / f64::from(detents);
    assert!(
        (ratio - 120.0).abs() < 5.0,
        "expected ~120 hi-res units per detent, got ratio={ratio}"
    );
}

#[test]
fn horizontal_and_vertical_scroll_independently_in_diagonal_motion() {
    let mut e = engine_horizontal();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 120, dy: 120 });

    let actions = run_for(&mut e, 1.0, 8333);
    let v = count_detents(&actions);
    let h = count_detents_h(&actions);

    assert!(v < 0, "vertical down should be negative, got {v}");
    assert!(h > 0, "horizontal right should be positive, got {h}");
}

#[test]
fn horizontal_scroll_speed_steps_pick_last_reached_distance() {
    let cfg = CoreConfig {
        horizontal_scroll: true,
        emit_hires_wheel: false,
        scroll_speed_steps: vec![
            SpeedStep {
                distance_units: 20,
                speed_detents_per_second: 2.0,
            },
            SpeedStep {
                distance_units: 100,
                speed_detents_per_second: 8.0,
            },
        ],
        ..CoreConfig::default()
    };

    let mut slow = engine_with(cfg.clone());
    slow.process(CoreInputEvent::MiddleDown);
    slow.process(CoreInputEvent::Motion { dx: 30, dy: 0 });
    let slow_detents = count_detents_h(&run_for(&mut slow, 1.0, 100_000)).abs();

    let mut fast = engine_with(cfg);
    fast.process(CoreInputEvent::MiddleDown);
    fast.process(CoreInputEvent::Motion { dx: 120, dy: 0 });
    let fast_detents = count_detents_h(&run_for(&mut fast, 1.0, 100_000)).abs();

    assert!(
        (1..=3).contains(&slow_detents),
        "expected roughly 2 detents, got {slow_detents}"
    );
    assert!(
        (7..=9).contains(&fast_detents),
        "expected roughly 8 detents, got {fast_detents}"
    );
}

#[test]
fn horizontal_offset_is_clamped_to_max_offset_units() {
    let mut e = engine_horizontal();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 100_000, dy: 0 });
    assert_eq!(e.offset_x_units(), 240);

    e.process(CoreInputEvent::Motion {
        dx: -1_000_000,
        dy: 0,
    });
    assert_eq!(e.offset_x_units(), -240);
}

#[test]
fn horizontal_motion_inside_deadzone_does_not_disturb_short_click() {
    let mut e = engine_horizontal();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 4, dy: 0 });
    e.process(CoreInputEvent::Motion { dx: -3, dy: 0 });
    let up = e.process(CoreInputEvent::MiddleUp);
    assert_eq!(up, vec![CoreAction::EmitMiddleClick]);
    assert_eq!(e.state(), EngineState::Idle);
}

#[test]
fn horizontal_release_during_scroll_returns_to_idle() {
    let mut e = engine_horizontal();
    e.process(CoreInputEvent::MiddleDown);
    e.process(CoreInputEvent::Motion { dx: 100, dy: 0 });
    let _ = run_for(&mut e, 0.2, 8333);

    let release = e.process(CoreInputEvent::MiddleUp);
    assert!(release.contains(&CoreAction::ExitScrollMode));
    assert!(!release.contains(&CoreAction::EmitMiddleClick));
    assert_eq!(e.state(), EngineState::Idle);
    assert_eq!(e.offset_x_units(), 0);
}
