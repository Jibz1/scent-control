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
fn the_toolbar_shows_the_log_and_opens_settings_as_does_command_comma() {
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(state());
    harness.run_steps(2);
    harness.get_by_label("Show Log").click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Show Log").is_none());
    harness.get_by_label("Hide Log").click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Show Log").is_some());

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

/// Shows the log, and lets its pane open.
fn show_log(harness: &mut egui_kittest::Harness<'static, Rig>) {
    use egui_kittest::kittest::Queryable as _;
    harness.run_steps(2);
    harness.get_by_label("Show Log").click();
    harness.run_steps(8);
}

#[test]
fn a_log_row_can_be_copied() {
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(state());
    show_log(&mut harness);
    let failure = |node: &egui_kittest::kittest::AccessKitNode<'_>| {
        node.label()
            .is_some_and(|said| said.ends_with("over Bluetooth: No answer."))
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
fn a_log_row_reads_as_a_sentence_and_opens_to_its_message() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    let mut harness = harness(state());
    show_log(&mut harness);
    // The state sends it twice at once: the first of the two rows.
    let row = harness
        .get_all_by_role_and_label(Role::Button, "Turned Bedroom on, over WiFi")
        .next()
        .unwrap();
    assert_eq!(row.accesskit_node().data().is_expanded(), Some(false));
    row.click();
    harness.run_steps(4);
    let row = harness
        .get_all_by_role_and_label(Role::Button, "Turned Bedroom on, over WiFi")
        .next()
        .unwrap();
    assert_eq!(row.accesskit_node().data().is_expanded(), Some(true));
    // The masked message itself is drawn under it, scrolled into view; a click closes
    // it again.
    row.click();
    harness.run_steps(2);
    let row = harness
        .get_all_by_role_and_label(Role::Button, "Turned Bedroom on, over WiFi")
        .next()
        .unwrap();
    assert_eq!(row.accesskit_node().data().is_expanded(), Some(false));
}

#[test]
fn the_log_pane_keeps_its_height_with_nothing_in_it() {
    let mut harness = harness(AppState::default());
    show_log(&mut harness);
    let log = egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("log"));
    let height = log.expect("the log pane is shown").size().y;
    assert!((height - log_view::OPEN_HEIGHT).abs() < 1.0, "{height}");
}

#[test]
fn the_sidebar_footer_shows_the_broker_state_in_short_and_reads_it_in_full_once() {
    use egui_kittest::kittest::Queryable as _;
    let mut state = state();
    state.broker = BrokerState::LoginRefused("Not authorized.".to_owned());
    let mut harness = harness(state);
    harness.run_steps(2);
    // One element reads the whole sentence, reason and all; the dot isn't read apart.
    let full = "Broker login refused: Not authorized.";
    assert_eq!(harness.get_all_by_value(full).count(), 1);
    assert!(harness.query_by_label("Status").is_none());
}

#[test]
fn add_on_a_diffuser_found_nearby_opens_the_sheet_and_hands_over_the_add() {
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(state());
    harness.run_steps(2);
    // Only the hit we don't know yet is listed; selecting it shows the way to add it.
    assert!(harness.query_by_label_contains("Known").is_none());
    harness.get_by_label("Diffuser, Signal unknown").click();
    harness.run_steps(2);
    let _ = harness.get_by_label("Diffuser is nearby");
    harness.get_by_label("Add Diffuser…").click();
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

    // Another add finishing doesn't take the selection.
    harness
        .state_mut()
        .screens
        .on_event(&crate::state::Event::Adding {
            ble_id: "ble-other".to_owned(),
            step: crate::add_flow::AddStep::Done(super::tests::TWO.to_owned()),
        });
    harness.run_steps(2);
    assert_eq!(harness.state().screens.selected.as_deref(), Some(ONE));

    // Once this one is done, it is the diffuser that is open.
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
/// part takes a few frames to measure what it holds.
fn open_schedule(harness: &mut egui_kittest::Harness<'static, Rig>) {
    use egui_kittest::kittest::Queryable as _;
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
    // Below the always-on and time zone boxes: scrolled to, as a person would.
    harness
        .get_by_label("Add a Test Time Starting in 3 Minutes")
        .scroll_to_me();
    harness.run_steps(4);
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

/// The open diffuser's state, as the hero reads it out: one element, its name and what
/// it is doing.
#[test]
fn the_hero_says_diffusing_or_off_with_the_intensity() {
    use egui_kittest::kittest::Queryable as _;
    let mut on = state();
    let one = on.devices.get_mut(ONE).unwrap();
    one.sent = None;
    one.readings
        .get_mut(&crate::state::Field::Intensity)
        .unwrap()
        .at = NOW - 20;
    let mut diffusing = harness(on.clone());
    diffusing.run_steps(2);
    let _ = diffusing.get_by_label("Bedroom, diffusing at Medium intensity");
    let _ = diffusing.get_by_label("Medium");

    let mut off = on;
    let one = off.devices.get_mut(ONE).unwrap();
    one.readings
        .get_mut(&crate::state::Field::Power)
        .unwrap()
        .value = 0;
    let mut stopped = harness(off);
    stopped.run_steps(2);
    let _ = stopped.get_by_label("Bedroom, off");
}

#[test]
fn the_hero_says_sending_in_orange_while_a_command_is_on_its_way() {
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    let mut calm = state();
    calm.devices.get_mut(ONE).unwrap().sent = None;
    let mut harness = harness(calm);
    harness.run_steps(2);
    harness.get_by_label("High").click();
    harness.run_steps(2);
    assert_eq!(harness.state().commands.len(), 1);
    let hero = harness.get_by(|node| {
        node.label()
            .is_some_and(|label| label.starts_with("Bedroom, sending"))
    });
    assert_eq!(
        hero.accesskit_node().label().as_deref(),
        Some("Bedroom, sending: diffusing at High intensity")
    );
    // In orange, the color of what is on its way.
    for dark in [false, true] {
        let palette = theme::Palette::for_dark(dark);
        let sending = super::hero::HeroState {
            sending: true,
            ..super::hero::HeroState::default()
        };
        assert_eq!(sending.state_words(), "Sending…");
        assert_eq!(sending.state_color(&palette), palette.orange);
    }
}

#[test]
fn the_sidebar_says_what_each_diffuser_is_doing() {
    use egui_kittest::kittest::Queryable as _;
    use scent_core::Online;
    let mut calm = state();
    calm.devices.get_mut(ONE).unwrap().sent = None;
    let one = &calm.devices[ONE];
    assert_eq!(
        super::device_list::second_line(one, Online::Online),
        "Diffusing at Medium"
    );
    let mut off = one.clone();
    off.readings
        .get_mut(&crate::state::Field::Power)
        .unwrap()
        .value = 0;
    assert_eq!(super::device_list::second_line(&off, Online::Online), "Off");
    assert_eq!(
        super::device_list::second_line(one, Online::Offline),
        "Not answering"
    );
    assert_eq!(
        super::device_list::second_line(one, Online::Waiting),
        "Waiting"
    );
    // In the sidebar, under the name; the unnamed diffuser goes by its model, or by
    // "Diffuser" and the end of its ID, never by the ID itself.
    calm.devices.get_mut(ONE).unwrap().heard_wifi = Some(NOW - 5);
    let mut harness = harness(calm);
    harness.run_steps(2);
    let _ = harness.get_by_label("Bedroom, Diffusing at Medium");
    let _ = harness.get_by_label("Diffuser CE02, Not pinged yet");
    assert!(harness.query_by_label_contains("TESTDEVICE02").is_none());
}

#[test]
fn the_log_is_hidden_by_default() {
    use egui_kittest::kittest::Queryable as _;
    assert!(!Screens::new(None).log.is_open());
    let mut harness = harness(state());
    harness.run_steps(2);
    assert!(harness.query_by_label("Hide Log").is_none());
    let _ = harness.get_by_label("Show Log");
    let log = egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("log"));
    assert!(log.is_none_or(|log| log.size().y < 1.0), "{log:?}");
}

#[test]
fn the_schedule_is_read_out_today_and_for_the_week() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use scent_core::{Entry, HhMm, Level};
    let mut state = state();
    // A time on Monday and Tuesday (today, at UTC-5), as well as always on.
    let device = state.devices.get_mut(ONE).unwrap();
    let profile = device.sent_profile.as_mut().unwrap();
    profile.entries.push(Entry {
        start: HhMm::parse("07:00").unwrap(),
        end: HhMm::parse("09:00").unwrap(),
        weekdays: vec![1, 2],
        level: Level::new(2).unwrap(),
        enable: true,
    });
    let mut harness = harness(state);
    harness.run_steps(2);
    // One element, named once, its value the day in words.
    let today = harness.get_by_role_and_label(Role::Image, "Today");
    assert_eq!(
        today.accesskit_node().value().as_deref(),
        Some("07:00 to 09:00 at Medium; always on at Low the rest of the day")
    );
    assert_eq!(harness.query_all_by_label("Today").count(), 1);
    // The editor's week grid reads the draft out day by day.
    harness.get_by_label("Edit Schedule…").click();
    harness.run_steps(6);
    let week = harness.get_by_role_and_label(Role::Image, "Week");
    assert_eq!(
        week.accesskit_node().value().as_deref(),
        Some(
            "Monday 07:00 to 09:00 at Medium; Tuesday 07:00 to 09:00 at Medium; \
             always on at Low the rest of the time"
        )
    );
}
