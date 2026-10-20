//! The screens driven through egui_kittest, as a person would use them: clicks, keys
//! and what accessibility sees. No hub: the commands they ask for are collected.

use std::path::Path;

use eframe::egui::{self, Key};

use super::Screens;
use super::tests::{NOW, ONE, state};
use super::{log_view, theme};
use crate::hub::{NO_BROKER_PASSWORD, UiCommand};
use crate::mqtt::BrokerSettings;
use crate::state::{AppState, BrokerState};

/// The screens with what they asked for, for a test harness to drive.
struct Rig {
    screens: Screens,
    state: AppState,
    commands: Vec<UiCommand>,
}

/// A harness that draws the screens in egui's fonts, without a hub. The first frame
/// installs the theme, which is in use from the next.
fn harness(state: AppState) -> egui_kittest::Harness<'static, Rig> {
    let mut frames = 0;
    let rig = Rig {
        screens: Screens::new(None),
        state,
        commands: Vec::new(),
    };
    egui_kittest::Harness::builder()
        .with_size(theme::WINDOW_SIZE)
        .with_os(egui::os::OperatingSystem::Mac)
        .build_ui_state(
            move |ui, rig: &mut Rig| {
                frames += 1;
                if frames == 1 {
                    theme::install(ui.ctx(), Path::new("/nonexistent"));
                    ui.ctx().request_repaint();
                    return;
                }
                let requests = rig
                    .screens
                    .show(ui, &rig.state, &BrokerSettings::default(), NOW);
                rig.commands.extend(requests.commands);
            },
            rig,
        )
}

#[test]
fn the_switch_sends_the_power_and_shows_it_while_it_goes() {
    use egui::accesskit::{Role, Toggled};
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    let mut state = state();
    state.devices.get_mut(ONE).unwrap().sent = None;
    let mut harness = harness(state);
    harness.run_steps(2);
    // On, as reported.
    let power = harness.get_by_role_and_label(Role::Switch, "Power");
    assert_eq!(power.accesskit_node().toggled(), Some(Toggled::True));
    harness.get_by_role_and_label(Role::Switch, "Power").click();
    harness.run_steps(2);
    assert!(
        matches!(
            harness.state().commands[..],
            [UiCommand::Send {
                cmd: scent_core::Command::TurnOn { on: false },
                pref: crate::route::TransportPref::Auto,
                ..
            }]
        ),
        "one command, to turn it off"
    );
    // Off while the command is on its way, though nothing says so yet.
    let power = harness.get_by_role_and_label(Role::Switch, "Power");
    assert_eq!(power.accesskit_node().toggled(), Some(Toggled::False));
}

#[test]
fn the_toolbar_hides_the_log_and_opens_settings_as_does_command_comma() {
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(state());
    harness.run_steps(2);
    harness.get_by_label("Hide Log").click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Hide Log").is_none());
    harness.get_by_label("Show Log").click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Hide Log").is_some());

    assert!(!harness.state().screens.settings.is_open());
    harness.get_by_label("Settings").click();
    harness.run_steps(2);
    assert!(harness.state().screens.settings.is_open());
    harness.get_by_label("Cancel").click();
    harness.run_steps(2);
    assert!(!harness.state().screens.settings.is_open());

    harness.key_press_modifiers(egui::Modifiers::COMMAND, Key::Comma);
    harness.run_steps(2);
    assert!(harness.state().screens.settings.is_open());
    assert!(harness.state().commands.is_empty());
}

#[test]
fn a_log_line_can_be_copied() {
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(state());
    harness.run_steps(2);
    let failure = |node: &egui_kittest::kittest::AccessKitNode<'_>| {
        node.value()
            .is_some_and(|line| line.ends_with("No answer."))
    };
    harness.get_by(failure).click_secondary();
    harness.run_steps(2);
    harness.get_by_label("Copy").click();
    harness.step();
    let copied = harness.output().platform_output.commands.iter().any(
        |command| matches!(command, egui::OutputCommand::CopyText(text) if text == "No answer."),
    );
    assert!(copied, "{:?}", harness.output().platform_output.commands);
}

#[test]
fn no_password_saved_is_a_notice_with_a_way_to_settings() {
    use egui_kittest::kittest::Queryable as _;
    let mut state = state();
    state.broker = BrokerState::Disconnected(NO_BROKER_PASSWORD.to_owned());
    let mut harness = harness(state);
    harness.run_steps(2);
    harness.get_by_label("Open Settings").click();
    harness.run_steps(2);
    assert!(harness.state().screens.settings.is_open());
}

#[test]
fn the_log_pane_keeps_its_height_with_nothing_in_it() {
    let mut harness = harness(AppState::default());
    harness.run_steps(3);
    let log = egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("log"));
    let height = log.expect("the log pane is shown").size().y;
    assert!((height - log_view::OPEN_HEIGHT).abs() < 1.0, "{height}");
}

#[test]
fn the_toolbar_shows_the_broker_state_in_short_and_reads_it_in_full() {
    use egui_kittest::kittest::Queryable as _;
    let mut state = state();
    state.broker = BrokerState::LoginRefused("Not authorized.".to_owned());
    let mut harness = harness(state);
    harness.run_steps(2);
    // The dot and the words both read the whole sentence, reason and all.
    let full = "Broker login refused: Not authorized.";
    assert_eq!(harness.get_all_by_value(full).count(), 2);
}

#[test]
fn add_on_a_diffuser_found_nearby_opens_the_sheet_and_hands_over_the_add() {
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(state());
    harness.run_steps(2);
    // Only the hit we don't know yet has the button.
    harness.get_by_label("Add…").click();
    harness.run_steps(2);
    let _ = harness.get_by_label("Add Diffuser");
    harness.get_by_label("Add").click();
    harness.run_steps(2);
    let [UiCommand::Add(request)] = &harness.state().commands[..] else {
        panic!("one Add");
    };
    assert_eq!(request.ble_id, "ble-9");
    assert_eq!(request.label, "Diffuser");
    assert!(request.wifi.is_none());

    // Once it is done, it is the diffuser that is open.
    harness
        .state_mut()
        .screens
        .on_event(&crate::state::Event::Adding {
            ble_id: "ble-9".to_owned(),
            step: crate::add_flow::AddStep::Done(super::tests::TWO.to_owned()),
        });
    harness.run_steps(2);
    assert_eq!(
        harness.state().screens.selected.as_deref(),
        Some(super::tests::TWO)
    );
}

/// Opens Edit Schedule on the open diffuser, and lets the sheet settle: its scrolling
/// part takes a few frames to measure what it holds. The log is hidden first, as the
/// Schedule box is below it in a window this size.
fn open_schedule(harness: &mut egui_kittest::Harness<'static, Rig>) {
    use egui_kittest::kittest::Queryable as _;
    harness.run_steps(2);
    harness.get_by_label("Hide Log").click();
    harness.run_steps(2);
    harness.get_by_label("Edit Schedule…").click();
    harness.run_steps(6);
}

#[test]
fn a_schedule_is_sent_only_once_it_is_confirmed() {
    use egui_kittest::kittest::Queryable as _;
    use scent_core::Profile;
    let mut harness = harness(state());
    open_schedule(&mut harness);
    assert!(harness.state().screens.schedule.is_open());
    harness
        .get_by_label("Add a Test Time Starting in 3 Minutes")
        .click();
    harness.run_steps(6);
    harness.get_by_label("Send Schedule").click();
    harness.run_steps(2);
    // Nothing goes until the question is answered.
    assert!(harness.state().commands.is_empty());
    let _ = harness.get_by_label("Replace Bedroom's stored schedule?");
    harness.get_by_label("Replace").click();
    harness.run_steps(2);
    let [UiCommand::SendSchedule { device_id, profile }] = &harness.state().commands[..] else {
        panic!("one schedule");
    };
    assert_eq!(device_id, ONE);
    // It started from the schedule this app sent: always on, at UTC-5. The test time
    // starts 3 minutes from now there (10:30 on a Tuesday).
    let Profile {
        always_on,
        entries,
        timezone_hours,
        ..
    } = profile;
    assert!(always_on.enable);
    assert_eq!(*timezone_hours, -5.0);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        (entries[0].start.to_string(), entries[0].end.to_string()),
        ("10:33".to_owned(), "10:36".to_owned())
    );
    assert_eq!(entries[0].weekdays, [2]);
    assert!(!harness.state().screens.schedule.is_open());
}

#[test]
fn send_schedule_waits_until_the_schedule_is_valid() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    let mut harness = harness(state());
    open_schedule(&mut harness);
    harness.state_mut().screens.schedule.draft_mut().offset = "east".to_owned();
    harness.run_steps(2);
    let _ = harness.get_by_label("Enter the offset from UTC in hours, such as -5 or 5.5.");
    let send = harness.get_by_role_and_label(Role::Button, "Send Schedule");
    assert!(send.accesskit_node().is_disabled());
    harness.get_by_label("Send Schedule").click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Replace").is_none());

    // Escape closes the sheet, sending nothing.
    harness.key_press(Key::Escape);
    harness.run_steps(2);
    assert!(!harness.state().screens.schedule.is_open());
    assert!(harness.state().commands.is_empty());
}

#[test]
fn escape_in_the_confirmation_goes_back_to_the_schedule() {
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(state());
    open_schedule(&mut harness);
    harness.get_by_label("Send Schedule").click();
    harness.run_steps(2);
    let _ = harness.get_by_label("Replace");
    harness.key_press(Key::Escape);
    harness.run_steps(2);
    assert!(harness.query_by_label("Replace").is_none());
    assert!(harness.state().screens.schedule.is_open());
    assert!(harness.state().commands.is_empty());
}
