// Ported from the tests in `packages/scent-core/src/schedule.rs`.

import Testing

@testable import ScentProtocol

private func off(_ n: Int) -> AlwaysOn { AlwaysOn(enable: false, level: level(n)) }

private func window(_ start: String, _ end: String, _ weekdays: [Int], _ n: Int) -> Entry {
  Entry(start: hhmm(start), end: hhmm(end), weekdays: weekdays, level: level(n), enable: true)
}

/// 2026-10-18 is a Sunday.
private let sundayMidnightUTC = 1_792_281_600

/// Ported from tools/tests/test_control_server.py: a Tuesday 10:24-10:27 window at UTC-5.
private func tuesdayWindow() -> Profile {
  Profile(
    alwaysOn: off(1), entries: [window("10:24", "10:27", [2], 3)], timezoneHours: -5.0,
    flag: .replace)
}

@Suite struct HhMmTests {
  @Test func parseAcceptsHhMmAndNothingElse() {
    for (text, hour, minute) in [("00:00", 0, 0), ("07:05", 7, 5), ("23:59", 23, 59)] {
      #expect(HhMm(parsing: text) == HhMm(hour: hour, minute: minute), "\(text)")
    }
    for bad in [
      "", "7:00", "24:00", "12:60", "12-30", "1200", "+7:00", "12:3", "12:300", " 7:00", "12:30 ",
      "ab:cd", "12:30\n", "１２:３０",
    ] {
      #expect(HhMm(parsing: bad) == nil, "\(bad.debugDescription)")
    }
  }

  @Test func displayIsTwoDigitAndParsesBack() {
    #expect(HhMm(hour: 7, minute: 5)!.description == "07:05")
    for text in ["00:00", "09:59", "12:30", "23:59"] {
      #expect(hhmm(text).description == text)
    }
  }

  @Test func timesOrderByTheClock() {
    #expect(hhmm("09:59") < hhmm("10:00"))
    #expect(hhmm("10:00") < hhmm("10:01"))
    #expect(hhmm("00:00") < hhmm("23:59"))
  }
}

@Suite struct LocalTimeTests {
  @Test func weekdayZeroIsSundayAndSixIsSaturday() {
    for (daysAfterSunday, weekday) in [(0, 0), (1, 1), (3, 3), (6, 6), (7, 0)] {
      let local = localWeekdayAndTime(
        nowUTC: sundayMidnightUTC + daysAfterSunday * 86_400, tzHours: 0)
      #expect(
        local.weekday == weekday && local.time == hhmm("00:00"),
        "\(daysAfterSunday) days after Sunday")
    }
    // 1970-01-01 was a Thursday, and the second before it a Wednesday.
    let epoch = localWeekdayAndTime(nowUTC: 0, tzHours: 0)
    #expect(epoch.weekday == 4 && epoch.time == hhmm("00:00"))
    let before = localWeekdayAndTime(nowUTC: -1, tzHours: 0)
    #expect(before.weekday == 3 && before.time == hhmm("23:59"))
  }

  @Test func localTimeFollowsTheOffsetIncludingHalfHours() {
    // Sunday 00:00 UTC is already Sunday 05:30 in India ...
    let india = localWeekdayAndTime(nowUTC: sundayMidnightUTC, tzHours: 5.5)
    #expect(india.weekday == 0 && india.time == hhmm("05:30"))
    // ... and still Saturday 20:30 in Newfoundland-style UTC-3:30.
    let newfoundland = localWeekdayAndTime(nowUTC: sundayMidnightUTC, tzHours: -3.5)
    #expect(newfoundland.weekday == 6 && newfoundland.time == hhmm("20:30"))
    // Seconds are dropped, not rounded.
    let late = localWeekdayAndTime(nowUTC: sundayMidnightUTC + 59, tzHours: 0)
    #expect(late.weekday == 0 && late.time == hhmm("00:00"))
  }

  @Test func absurdInputsGiveAValidTimeInsteadOfCrashing() {
    for (at, tz) in [
      (Int.max, Double.infinity), (Int.min, -Double.infinity), (Int.max, 1e300), (0, Double.nan),
      (Int.min, -1e300), (Int.max, 14.0), (Int.min, -14.0),
    ] {
      let (weekday, time) = localWeekdayAndTime(nowUTC: at, tzHours: tz)
      #expect(
        (0...6).contains(weekday) && (0..<24).contains(time.hour) && (0..<60).contains(time.minute))
    }
    // As in Rust, a NaN offset counts as zero.
    #expect(
      localWeekdayAndTime(nowUTC: sundayMidnightUTC, tzHours: .nan)
        == localWeekdayAndTime(nowUTC: sundayMidnightUTC, tzHours: 0))
  }
}

@Suite struct ExpectedStateTests {
  @Test func expectedStateDuringAndAfterAWindow() {
    let p = tuesdayWindow()
    // 2026-10-20T15:25:00Z is Tuesday 10:25 at UTC-5.
    #expect(
      expectedState(p, nowUTC: 1_792_509_900)
        == Expected(
          power: true, level: level(3), source: .entry(start: hhmm("10:24"), end: hhmm("10:27")),
          until: hhmm("10:27")))
    // Three minutes later the window is over and the always-on entry (off) decides.
    #expect(
      expectedState(p, nowUTC: 1_792_510_080)
        == Expected(power: false, level: nil, source: .alwaysOn, until: nil))
  }

  @Test func aWindowIncludesItsStartMinuteAndExcludesItsEndMinute() {
    let p = tuesdayWindow()
    let at152359 = 1_792_509_839
    #expect(!expectedState(p, nowUTC: at152359).power)  // 10:23:59
    #expect(expectedState(p, nowUTC: at152359 + 1).power)  // 10:24:00
    #expect(expectedState(p, nowUTC: 1_792_510_019).power)  // 10:26:59
    #expect(!expectedState(p, nowUTC: 1_792_510_020).power)  // 10:27:00
  }

  @Test func expectedStateUsesTheLocalWeekdayAcrossMidnightUTC() {
    // Wed 2026-10-21 03:00 UTC is Tue 22:00 at UTC-5
    let p = Profile(
      alwaysOn: off(1),
      entries: [
        Entry(
          start: hhmm("21:30"), end: hhmm("22:30"), weekdays: [2], level: level(1), enable: true)
      ],
      timezoneHours: -5.0, flag: .replace)
    let e = expectedState(p, nowUTC: 1_792_551_600)  // 2026-10-21T03:00:00Z
    #expect(e.power && e.until == hhmm("22:30"))
  }

  @Test func anEntryOnAnotherWeekdayOrSwitchedOffIsIgnored() {
    var p = tuesdayWindow()
    p.entries[0].weekdays = [0, 1, 3, 4, 5, 6]
    #expect(!expectedState(p, nowUTC: 1_792_509_900).power)

    p = tuesdayWindow()
    p.entries[0].enable = false
    #expect(!expectedState(p, nowUTC: 1_792_509_900).power)
  }

  @Test func theFirstActiveEntryWins() {
    let p = Profile(
      alwaysOn: off(1),
      entries: [window("10:00", "11:00", [2], 2), window("10:20", "10:30", [2], 3)],
      timezoneHours: -5.0, flag: .replace)
    let e = expectedState(p, nowUTC: 1_792_509_900)  // Tuesday 10:25
    #expect(e.level == level(2))
    #expect(e.until == hhmm("11:00"))
  }

  @Test func outsideEveryWindowTheAlwaysOnEntryDecides() {
    let on = Profile(
      alwaysOn: AlwaysOn(enable: true, level: level(2)),
      entries: [window("10:24", "10:27", [2], 3)],
      timezoneHours: -5.0, flag: .replace)
    #expect(
      expectedState(on, nowUTC: 1_792_510_080)
        == Expected(power: true, level: level(2), source: .alwaysOn, until: nil))
    // An off always-on entry reports no level, whatever level it stores.
    let e = expectedState(
      Profile(alwaysOn: off(3), entries: [], timezoneHours: 0, flag: .replace), nowUTC: 0)
    #expect(!e.power && e.level == nil)
  }

  @Test func theFlagOnlyDecidesWhatIsSentNotWhatIsExpected() {
    // With flag 0 the diffuser keeps the timed entries it already has, so the table in
    // `entries` still describes it.
    var p = tuesdayWindow()
    p.flag = .alwaysOnOnly
    #expect(expectedState(p, nowUTC: 1_792_509_900).power)
  }
}

@Suite struct ProfilePayloadTests {
  @Test func payloadShapeMatchesTheFirmware() {
    let p = Profile(
      alwaysOn: off(1),
      entries: [
        Entry(
          start: hhmm("07:00"), end: hhmm("09:00"), weekdays: [1, 2, 3, 4, 5], level: level(1),
          enable: true)
      ],
      timezoneHours: -5.0, flag: .replace)
    #expect(
      p.payloadJSON()
        == #"{"profile":{"schedule":[{"index":1,"startTime":"00:00","endTime":"23:59","weekdays":[0,1,2,3,4,5,6],"workingState":{"level":1,"fan":0,"led":0,"timer":0},"enable":false,"isFactory":true},{"startTime":"07:00","endTime":"09:00","weekdays":[1,2,3,4,5],"workingState":{"level":1,"fan":0,"led":0,"timer":0},"enable":true}]},"timezone":-5.0,"schedule_flag":1}"#
    )
  }

  @Test func payloadKeysComeOutInTheOrderTheAppWritesThem() {
    // Expected text from tools/scentiment_ble.py (build_profile).
    let p = Profile(
      alwaysOn: off(1), entries: [window("07:00", "09:00", [1, 2, 3, 4, 5], 3)],
      timezoneHours: -5.0,
      flag: .replace)
    #expect(
      p.payloadJSON()
        == #"{"profile":{"schedule":[{"index":1,"startTime":"00:00","endTime":"23:59","weekdays":[0,1,2,3,4,5,6],"workingState":{"level":1,"fan":0,"led":0,"timer":0},"enable":false,"isFactory":true},{"startTime":"07:00","endTime":"09:00","weekdays":[1,2,3,4,5],"workingState":{"level":3,"fan":0,"led":0,"timer":0},"enable":true}]},"timezone":-5.0,"schedule_flag":1}"#
    )
  }

  @Test func theFactoryEntryCarriesTheAlwaysOnSettings() throws {
    let p = Profile(
      alwaysOn: AlwaysOn(enable: true, level: level(3)), entries: [], timezoneHours: 5.5,
      flag: .replace)
    let payload = p.payload
    let schedule = try #require(payload["profile"]?["schedule"]?.array)
    // No timed entries: the schedule is only the factory entry, and flag 1 clears the rest.
    #expect(schedule.count == 1)
    #expect(schedule[0]["enable"]?.text == "true")
    #expect(schedule[0]["workingState"]?["level"]?.int64 == 3)
    #expect(schedule[0]["isFactory"]?.text == "true")
    #expect(payload["timezone"]?.text == "5.5")
    #expect(payload["schedule_flag"]?.int64 == 1)
  }

  @Test func alwaysOnOnlySendsJustTheFactoryEntryWithFlagZero() {
    let p = Profile(
      alwaysOn: AlwaysOn(enable: true, level: level(2)),
      entries: [window("07:00", "09:00", [1], 3)],
      timezoneHours: -5.0, flag: .alwaysOnOnly)
    #expect(
      p.payloadJSON()
        == #"{"profile":{"schedule":[{"index":1,"startTime":"00:00","endTime":"23:59","weekdays":[0,1,2,3,4,5,6],"workingState":{"level":2,"fan":0,"led":0,"timer":0},"enable":true,"isFactory":true}]},"timezone":-5.0,"schedule_flag":0}"#
    )
  }
}

@Suite struct TrialEntryTests {
  // Ported from tools/tests/test_scentiment_ble.py: Monday 14:58, start in 3, last 4, level 3.
  @Test func trialEntryStartsSoonAndStaysOnOneDay() throws {
    #expect(
      try trialEntry(
        weekday: 1, now: hhmm("14:58"), startInMinutes: 3, durationMinutes: 4, level: level(3))
        == Entry(
          start: hhmm("15:01"), end: hhmm("15:05"), weekdays: [1], level: level(3), enable: true))
    let error = #expect(throws: TrialError.self) {
      try trialEntry(
        weekday: 1, now: hhmm("23:58"), startInMinutes: 3, durationMinutes: 4, level: level(3))
    }
    #expect(
      error?.message
        == "the test schedule would cross midnight; pick a start time that ends before 23:59")
  }

  @Test func trialEntryMayEndAt2359ButNotAtMidnight() throws {
    let e = try trialEntry(
      weekday: 0, now: hhmm("23:50"), startInMinutes: 0, durationMinutes: 9, level: level(1))
    #expect(e.start == hhmm("23:50") && e.end == hhmm("23:59"))
    let error = #expect(throws: TrialError.self) {
      try trialEntry(
        weekday: 0, now: hhmm("23:50"), startInMinutes: 0, durationMinutes: 10, level: level(1))
    }
    #expect(error?.message.contains("midnight") == true)
  }

  @Test func trialEntrySurvivesHugeArguments() {
    for (start, duration) in [
      (Int(UInt16.max), Int(UInt16.max)), (Int.max, Int.max), (0, Int.max), (Int.max, 0),
    ] {
      let error = #expect(throws: TrialError.self) {
        try trialEntry(
          weekday: 0, now: hhmm("23:59"), startInMinutes: start, durationMinutes: duration,
          level: level(1))
      }
      #expect(error?.message.contains("midnight") == true)
    }
  }

  /// Swift only: the counts are `Int`, so a negative one has to be refused.
  @Test func trialEntryRefusesNegativeCounts() {
    for (start, duration) in [(-1, 4), (3, -1), (Int.min, Int.min)] {
      #expect(throws: TrialError.self) {
        try trialEntry(
          weekday: 0, now: hhmm("12:00"), startInMinutes: start, durationMinutes: duration,
          level: level(1))
      }
    }
  }

  @Test func aTrialEntryIsActiveWhenItSaysSo() throws {
    // The Mon 14:58 trial above, run at UTC: on at 15:01, off at 15:05.
    let now = sundayMidnightUTC + 86_400 + (14 * 60 + 58) * 60
    let (weekday, time) = localWeekdayAndTime(nowUTC: now, tzHours: 0)
    let entry = try trialEntry(
      weekday: weekday, now: time, startInMinutes: 3, durationMinutes: 4, level: level(3))
    let p = Profile(alwaysOn: off(1), entries: [entry], timezoneHours: 0, flag: .replace)
    #expect(!expectedState(p, nowUTC: now).power)
    #expect(expectedState(p, nowUTC: now + 3 * 60).power)
    #expect(!expectedState(p, nowUTC: now + 7 * 60).power)
  }
}
