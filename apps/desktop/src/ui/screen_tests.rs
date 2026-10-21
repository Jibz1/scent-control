//! The screens driven through egui_kittest, as a person would use them: clicks, keys
//! and what accessibility sees. No hub: the commands they ask for are collected.

use std::path::Path;

use eframe::egui::{self, Key};

use super::Screens;
use super::tests::{NOW, ONE, state};
use super::{log_view, theme};
use crate::hub::{NO_BROKER_PASSWORD, UiCommand};
use crate::mqtt::BrokerSettings;
use crate::state::{AppState, BrokerState, Edit};

/// The screens with what they asked for, for a test harness to drive. The edits are made
/// to the state as the app makes them.
struct Rig {
    screens: Screens,
    state: AppState,
    commands: Vec<UiCommand>,
    edits: Vec<Edit>,
}

/// A harness that draws the screens in egui's fonts, without a hub. The first frame
/// installs the theme, which is in use from the next.
fn harness(state: AppState) -> egui_kittest::Harness<'static, Rig> {
    let mut frames = 0;
    let rig = Rig {
        screens: Screens::new(None),
        state,
        commands: Vec::new(),
        edits: Vec::new(),
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
                for edit in requests.edits {
                    rig.state.edit(edit.clone());
                    rig.edits.push(edit);
                }
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
    // The masked message itself is drawn under it, in view in the log pane, and read
    // out as the row's message.
    let message = harness.get_by_value(r#"{"action":"TURN_ON","payload":{"on":1}}"#);
    let pane = egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("log"))
        .expect("the log pane")
        .outer_rect;
    let shown = message.rect();
    assert!(
        pane.top() <= shown.top() && shown.bottom() <= pane.bottom(),
        "{shown:?} not in {pane:?}"
    );
    // A click closes it again.
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
    harness.get_by_label("Diffuser, signal unknown").click();
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

#[test]
fn a_diffuser_on_our_broker_can_be_set_up_again() {
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(state());
    harness.run_steps(2);
    // Bedroom has been heard on the broker: no notice to add it, and in Connection the
    // way to point it at the broker in Settings again.
    assert!(harness.query_by_label("Add to Broker…").is_none());
    let _ = harness.get_by_label("Points it at the broker in Settings and restarts it.");
    // At the foot of the pane: scrolled to, as a person would.
    harness.get_by_label("Set Up Again…").scroll_to_me();
    harness.run_steps(4);
    harness.get_by_label("Set Up Again…").click();
    harness.run_steps(2);
    // The add sheet, for that diffuser, with the same add as any other.
    assert!(harness.state().screens.add_sheet.is_for("ble-1"));
    let _ = harness.get_by_label("Add Diffuser");
    assert!(harness.state().commands.is_empty());
    harness.get_by_label("Add").click();
    harness.run_steps(2);
    let [UiCommand::Add(request)] = &harness.state().commands[..] else {
        panic!("one Add");
    };
    assert_eq!(request.ble_id, "ble-1");
    assert_eq!(request.label, "Bedroom");
    assert!(request.wifi.is_none());
}

#[test]
fn a_diffuser_not_yet_on_our_broker_keeps_add_to_broker() {
    use egui_kittest::kittest::Queryable as _;
    // The second diffuser, never heard on the broker: known over Bluetooth, and not.
    for ble_id in [Some("ble-2"), None] {
        let mut state = state();
        state.devices.get_mut(super::tests::TWO).unwrap().ble_id = ble_id.map(str::to_owned);
        let mut screens = harness(state);
        screens.run_steps(2);
        screens
            .get_by_label("Diffuser CE02, not pinged yet")
            .click();
        screens.run_steps(2);
        assert!(
            screens.query_by_label("Set Up Again…").is_none(),
            "{ble_id:?}"
        );
        match ble_id {
            Some(ble_id) => {
                screens.get_by_label("Add to Broker…").click();
                screens.run_steps(2);
                assert!(screens.state().screens.add_sheet.is_for(ble_id));
            }
            // Nothing to set it up by.
            None => assert!(screens.query_by_label("Add to Broker…").is_none()),
        }
    }
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
    use egui::accesskit::Role;
    let _ = diffusing.get_by_role_and_label(Role::Group, "Bedroom, diffusing at Medium intensity");
    let _ = diffusing.get_by_label("Medium");

    let mut off = on;
    let one = off.devices.get_mut(ONE).unwrap();
    one.readings
        .get_mut(&crate::state::Field::Power)
        .unwrap()
        .value = 0;
    let mut stopped = harness(off);
    stopped.run_steps(2);
    let _ = stopped.get_by_role_and_label(Role::Group, "Bedroom, off");
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
    // Read as one sentence: the words after the name in lower case.
    let _ = harness.get_by_label("Bedroom, diffusing at Medium");
    let _ = harness.get_by_label("Diffuser CE02, not pinged yet");
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

#[test]
fn after_a_relaunch_the_schedule_sent_still_shows_today_and_what_to_expect() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use scent_core::{Entry, HhMm, Level};
    let dir = crate::store::TempDir::new("screens-after-relaunch");
    let store = crate::store::Store {
        dir: dir.path().to_owned(),
    };
    let mut before = state();
    // A time this morning, Tuesday at UTC-5, as well as always on.
    let profile = before
        .devices
        .get_mut(ONE)
        .unwrap()
        .sent_profile
        .as_mut()
        .unwrap();
    profile.entries.push(Entry {
        start: HhMm::parse("07:00").unwrap(),
        end: HhMm::parse("09:00").unwrap(),
        weekdays: vec![2],
        level: Level::new(2).unwrap(),
        enable: true,
    });
    store.save(&before, &BrokerSettings::default()).unwrap();

    // The next run starts from the file alone.
    let mut harness = harness(store.load().state);
    harness.run_steps(2);
    let today = harness.get_by_role_and_label(Role::Image, "Today");
    assert_eq!(
        today.accesskit_node().value().as_deref(),
        Some("07:00 to 09:00 at Medium; always on at Low the rest of the day")
    );
    let _ = harness.get_by_label(
        "Based on the schedule this app last sent, it should be on at Low (always-on entry).",
    );
    assert!(harness.query_by_label("Nothing yet").is_none());
}

#[test]
fn an_opened_row_is_brought_into_view_once_and_let_go_when_filtered_out() {
    use egui_kittest::kittest::Queryable as _;
    let mut state = state();
    state.log.push_back(crate::state::LogEntry {
        at: NOW,
        device_id: Some(super::tests::TWO.to_owned()),
        via: None,
        direction: scent_core::Direction::ToDevice,
        action: "Bluetooth TURN_ON".to_owned(),
        text: "No answer.".to_owned(),
        sent: false,
    });
    let mut harness = harness(state);
    show_log(&mut harness);
    // Opened by a click: brought into view on the next frame, and only then.
    harness
        .get_all_by_label("Turned Bedroom on, over WiFi")
        .next()
        .unwrap()
        .click();
    harness.run_steps(2);
    assert!(!harness.state().screens.log.is_revealing());
    // Opened while it is filtered out: there is nothing to bring into view.
    let rig = harness.state_mut();
    rig.screens.log.as_open(&rig.state, |entry| {
        entry.device_id.as_deref() == Some(super::tests::TWO)
    });
    harness.get_by_label("Only the selected diffuser").click();
    harness.run_steps(2);
    assert!(!harness.state().screens.log.is_revealing());
}

#[test]
fn the_schedule_editor_calls_an_unnamed_diffuser_by_its_display_name() {
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(state());
    harness.run_steps(2);
    harness
        .get_by_label("Diffuser CE02, not pinged yet")
        .click();
    harness.run_steps(2);
    harness.get_by_label("Edit Schedule…").click();
    harness.run_steps(6);
    let _ = harness.get_by(|node| {
        node.value()
            .is_some_and(|text| text.starts_with("Diffuser CE02's timetable."))
    });
    harness.get_by_label("Send Schedule").click();
    harness.run_steps(2);
    let _ = harness.get_by_label("Replace Diffuser CE02's stored schedule?");
    // The raw ID shows only as the Device ID in Details, behind the sheet.
    let raw = |text: &str| text.contains("TESTDEVICE02") && text != "TESTDEVICE02";
    let named_by_id = harness.query_by(|node| {
        node.label().is_some_and(|text| raw(&text)) || node.value().is_some_and(|text| raw(&text))
    });
    assert!(named_by_id.is_none());
}

/// Types `text` over what the Details field `label` holds and leaves it with `key`:
/// Return to commit, Escape to put it back.
fn type_over(
    harness: &mut egui_kittest::Harness<'static, Rig>,
    label: &str,
    text: &str,
    key: Option<Key>,
) {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .focus();
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    if text.is_empty() {
        harness.key_press(Key::Backspace);
    } else {
        harness
            .get_by_role_and_label(Role::TextInput, label)
            .type_text(text);
    }
    harness.run_steps(2);
    if let Some(key) = key {
        harness.key_press(key);
        harness.run_steps(2);
    }
}

/// Whether a node is labelled with a sentence that starts with `start`.
fn starts(start: &'static str) -> impl Fn(&egui_kittest::kittest::AccessKitNode<'_>) -> bool {
    move |node| node.label().is_some_and(|label| label.starts_with(start))
}

/// The state with the first diffuser off, as it last reported, and nothing on its way.
fn calm() -> AppState {
    let mut state = state();
    state.devices.get_mut(ONE).unwrap().sent = None;
    state
}

#[test]
fn a_rename_in_details_shows_everywhere_at_once() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(calm());
    harness.run_steps(2);
    let name = harness.get_by_role_and_label(Role::TextInput, "Name");
    assert_eq!(name.value().as_deref(), Some("Bedroom"));

    type_over(&mut harness, "Name", "  Bedside ", Some(Key::Enter));
    let rig = harness.state();
    assert_eq!(rig.state.devices[ONE].label, "Bedside");
    assert_eq!(rig.edits.len(), 1, "{:?}", rig.edits);
    // The header, the hero, the sidebar and the field itself, all at once.
    let _ = harness.get_by_label("Bedside");
    let _ = harness.get_by(|node| node.role() == Role::Group && starts("Bedside, ")(node));
    assert!(harness.query_by(starts("Bedroom")).is_none());
    let sidebar = harness.get_all_by(starts("Bedside, ")).count();
    assert_eq!(sidebar, 2, "the sidebar row and the hero");
    let name = harness.get_by_role_and_label(Role::TextInput, "Name");
    assert_eq!(name.value().as_deref(), Some("Bedside"));
    // The log's sentences.
    show_log(&mut harness);
    let _ = harness
        .get_all_by_label("Turned Bedside on, over WiFi")
        .next()
        .expect("the log says the new name");
}

#[test]
fn a_renamed_diffuser_goes_by_its_new_name_in_the_sheets() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(calm());
    harness.run_steps(2);
    type_over(&mut harness, "Name", "Bedside", Some(Key::Tab));
    assert_eq!(harness.state().state.devices[ONE].label, "Bedside");
    // Edit Schedule's question.
    open_schedule(&mut harness);
    harness.get_by_label("Send Schedule").click();
    harness.run_steps(2);
    let _ = harness.get_by_label("Replace Bedside's stored schedule?");
    harness.key_press(Key::Escape);
    harness.run_steps(2);
    harness.key_press(Key::Escape);
    harness.run_steps(2);
    // Set Up Again… fills the add sheet in with it.
    harness.get_by_label("Set Up Again…").scroll_to_me();
    harness.run_steps(4);
    harness.get_by_label("Set Up Again…").click();
    harness.run_steps(2);
    assert!(harness.state().screens.add_sheet.is_open());
    // The sheet's Name field, and Details' behind it.
    let names: Vec<_> = harness
        .get_all_by_role_and_label(Role::TextInput, "Name")
        .map(|field| field.value())
        .collect();
    assert_eq!(
        names,
        [Some("Bedside".to_owned()), Some("Bedside".to_owned())]
    );
}

#[test]
fn escape_puts_the_name_back_and_changes_nothing() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(calm());
    harness.run_steps(2);
    type_over(&mut harness, "Name", "Kitchen", Some(Key::Escape));
    let rig = harness.state();
    assert_eq!(rig.state.devices[ONE].label, "Bedroom");
    assert!(rig.edits.is_empty(), "{:?}", rig.edits);
    let name = harness.get_by_role_and_label(Role::TextInput, "Name");
    assert_eq!(name.value().as_deref(), Some("Bedroom"));
}

#[test]
fn an_empty_name_goes_back_to_the_model() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    let mut harness = harness(calm());
    harness.run_steps(2);
    type_over(&mut harness, "Name", "", Some(Key::Enter));
    assert_eq!(harness.state().state.devices[ONE].label, ONE);
    // The model's name shows, and stands in the empty field.
    let _ = harness.get_by(|node| node.role() == Role::Group && starts("Air 2, ")(node));
    let name = harness.get_by_role_and_label(Role::TextInput, "Name");
    assert_eq!(name.value().as_deref(), Some(""));
    assert_eq!(name.accesskit_node().placeholder(), Some("Air 2"));
    assert!(harness.query_by_label_contains("TESTDEVICE01,").is_none());
}

#[test]
fn a_name_being_typed_is_kept_when_another_diffuser_is_chosen() {
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(calm());
    harness.run_steps(2);
    type_over(&mut harness, "Name", "Bedside", None);
    // A click on the other diffuser ends the typing: the name goes to the one it was for.
    harness
        .get_by_label("Diffuser CE02, not pinged yet")
        .click();
    harness.run_steps(2);
    let rig = harness.state();
    assert_eq!(rig.screens.selected.as_deref(), Some(super::tests::TWO));
    assert_eq!(rig.state.devices[ONE].label, "Bedside");
    assert_eq!(
        rig.state.devices[super::tests::TWO].label,
        super::tests::TWO
    );
}

#[test]
fn a_room_typed_in_details_groups_the_sidebar() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(calm());
    harness.run_steps(2);
    // No rooms yet: the one "Diffusers" section of before.
    let _ = harness.get_by_label("Diffusers");
    assert!(harness.query_by_label("No Room").is_none());

    type_over(&mut harness, "Room", "upstairs ", Some(Key::Enter));
    assert_eq!(
        harness.state().state.devices[ONE].room.as_deref(),
        Some("upstairs")
    );
    assert!(harness.query_by_label("Diffusers").is_none());
    // The room, then the diffusers in no room, each heading over its diffuser.
    let top = |harness: &egui_kittest::Harness<'static, Rig>, node: &str| {
        harness.get_by_label(node).rect().top()
    };
    let row = |harness: &egui_kittest::Harness<'static, Rig>, start| {
        let row = |node: &egui_kittest::kittest::AccessKitNode<'_>| {
            node.role() == Role::Button && starts(start)(node)
        };
        harness.get_by(row).rect().top()
    };
    let order = [
        top(&harness, "upstairs"),
        row(&harness, "Bedroom, "),
        top(&harness, "No Room"),
        row(&harness, "Diffuser CE02, "),
    ];
    assert!(order.is_sorted(), "{order:?}");
}

#[test]
fn a_room_in_use_can_be_picked_from_the_list() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;
    let mut state = calm();
    state.devices.get_mut(ONE).unwrap().room = Some("Upstairs".to_owned());
    let mut harness = harness(state);
    harness.run_steps(2);
    harness
        .get_by_label("Diffuser CE02, not pinged yet")
        .click();
    harness.run_steps(2);
    let room = harness.get_by_role_and_label(Role::TextInput, "Room");
    assert_eq!(room.value().as_deref(), Some(""));
    harness
        .get_by_role_and_label(Role::Button, "Choose a Room")
        .click();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::Button, "Upstairs")
        .click();
    harness.run_steps(2);
    let rig = harness.state();
    assert_eq!(
        rig.state.devices[super::tests::TWO].room.as_deref(),
        Some("Upstairs")
    );
    // Both are in it now: one section, and no "No Room".
    assert!(harness.query_by_label("No Room").is_none());
    let room = harness.get_by_role_and_label(Role::TextInput, "Room");
    assert_eq!(room.value().as_deref(), Some("Upstairs"));
}

#[test]
fn set_up_again_keeps_the_room_it_is_in() {
    use egui_kittest::kittest::Queryable as _;
    let mut state = state();
    state.devices.get_mut(ONE).unwrap().room = Some("Upstairs".to_owned());
    let mut harness = harness(state);
    harness.run_steps(2);
    harness.get_by_label("Set Up Again…").scroll_to_me();
    harness.run_steps(4);
    harness.get_by_label("Set Up Again…").click();
    harness.run_steps(2);
    harness.get_by_label("Add").click();
    harness.run_steps(2);
    let [UiCommand::Add(request)] = &harness.state().commands[..] else {
        panic!("one Add");
    };
    assert_eq!(
        (request.label.as_str(), request.room.as_str()),
        ("Bedroom", "Upstairs")
    );
}

#[test]
fn set_up_again_right_after_typing_a_name_opens_with_that_name() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(calm());
    harness.run_steps(2);
    // Typed, and not left with Return: the click on the button ends the typing.
    type_over(&mut harness, "Name", "Bedside", None);
    harness.get_by_label("Set Up Again…").scroll_to_me();
    harness.run_steps(4);
    harness.get_by_label("Set Up Again…").click();
    harness.run_steps(2);
    assert_eq!(harness.state().state.devices[ONE].label, "Bedside");
    let names: Vec<_> = harness
        .get_all_by_role_and_label(Role::TextInput, "Name")
        .map(|field| field.value())
        .collect();
    assert_eq!(
        names,
        [Some("Bedside".to_owned()), Some("Bedside".to_owned())]
    );
    harness.get_by_label("Add").click();
    harness.run_steps(2);
    let [UiCommand::Add(request)] = &harness.state().commands[..] else {
        panic!("one Add");
    };
    assert_eq!(request.label, "Bedside");
}

#[test]
fn add_to_broker_right_after_typing_a_room_opens_with_that_room() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;
    let mut state = calm();
    let two = state.devices.get_mut(super::tests::TWO).unwrap();
    two.ble_id = Some("ble-2".to_owned());
    let mut harness = harness(state);
    harness.run_steps(2);
    harness
        .get_by_label("Diffuser CE02, not pinged yet")
        .click();
    harness.run_steps(2);
    // The notice's button is above Details: it opens the sheet in the frame the typing ends.
    type_over(&mut harness, "Room", "Office", None);
    harness.get_by_label("Add to Broker…").click();
    harness.run_steps(2);
    assert!(harness.state().screens.add_sheet.is_for("ble-2"));
    let rooms: Vec<_> = harness
        .get_all_by_role_and_label(Role::TextInput, "Room")
        .map(|field| field.value())
        .collect();
    assert_eq!(
        rooms,
        [Some("Office".to_owned()), Some("Office".to_owned())]
    );
}

#[test]
fn switching_to_another_app_while_typing_ends_nothing() {
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(calm());
    harness.run_steps(2);
    type_over(&mut harness, "Name", "Bedsi", None);
    harness.input_mut().focused = false;
    harness.run_steps(2);
    assert!(harness.state().edits.is_empty());
    // Back in the app, the typing goes on, and Return ends it.
    harness.input_mut().focused = true;
    harness.run_steps(2);
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Name")
        .type_text("de");
    harness.run_steps(2);
    harness.key_press(Key::Enter);
    harness.run_steps(2);
    assert_eq!(harness.state().state.devices[ONE].label, "Bedside");
}

#[test]
fn the_list_of_rooms_opens_without_ending_the_typing() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    let mut state = calm();
    state.devices.get_mut(super::tests::TWO).unwrap().room = Some("Upstairs".to_owned());
    let mut harness = harness(state);
    harness.run_steps(2);
    type_over(&mut harness, "Room", "Up", None);
    harness
        .get_by_role_and_label(Role::Button, "Choose a Room")
        .click();
    harness.run_steps(2);
    // Nothing half typed went into the state, and the field still has the typing.
    assert!(harness.state().edits.is_empty());
    let room = harness.get_by_role_and_label(Role::TextInput, "Room");
    assert!(room.accesskit_node().is_focused());
    harness
        .get_by_role_and_label(Role::Button, "Upstairs")
        .click();
    harness.run_steps(2);
    let picked = Edit::Room {
        device_id: ONE.to_owned(),
        room: "Upstairs".to_owned(),
    };
    assert_eq!(harness.state().edits, [picked]);
}

#[test]
fn what_is_being_typed_is_kept_when_the_app_quits() {
    let mut harness = harness(calm());
    harness.run_steps(2);
    type_over(&mut harness, "Name", "Bedside", None);
    let edits = harness.state_mut().screens.end_typing();
    let typed = Edit::Name {
        device_id: ONE.to_owned(),
        name: "Bedside".to_owned(),
    };
    assert_eq!(edits, [typed]);
}
