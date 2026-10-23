import Foundation
import ScentProtocol

// The copy of a diffuser's last schedule that the app keeps on our own broker, so that
// every copy of the app, the Mac's and the phone's, knows it. Ported from
// `apps/desktop/src/shared_schedule.rs`. A diffuser can't say what schedule it holds, so
// the app remembers the last one it sent, and stores it as a retained message at
// `scent-control/v1/schedule/<deviceId>`. The newer copy wins. The payload is
// `{"v":1,"profile":<the schedule as the saved file keeps it>,"sent_at":<unix seconds>}`:
// times, days, levels and a time zone offset, nothing secret.

/// The only version of the payload this app writes and reads.
private let sharedVersion = 1

/// The most times a copy may hold: the largest table any firmware keeps, less its always-on
/// entry.
let mostTimes = 99

/// How far ahead of the reader's clock a copy's `sent_at` may be.
private let aheadAllowedSeconds = 86_400

struct SharedSchedule: Equatable {
  var profile: Profile
  /// Seconds since 1970.
  var sentAt: Int
}

/// Why a stored schedule can't be used.
enum Unreadable: Error, Equatable {
  /// Its `v` is not 1: another version of the app wrote it.
  case version
  /// It isn't JSON, or not ours, or its schedule can't be.
  case invalid
}

private struct Payload: Codable {
  var v: Int
  var profile: StoredProfile
  var sentAt: Int

  enum CodingKeys: String, CodingKey {
    case v, profile
    case sentAt = "sent_at"
  }
}

/// The payload for `profile`, sent at `sentAt`. Nil when the zone's offset isn't a number.
func encodeShared(_ profile: Profile, sentAt: Int) -> Data? {
  guard profile.timezoneHours.isFinite else { return nil }
  let encoder = JSONEncoder()
  encoder.outputFormatting = .sortedKeys
  return try? encoder.encode(Payload(v: sharedVersion, profile: StoredProfile(profile), sentAt: sentAt))
}

/// The schedule in a payload read at `now`. The version is read first, so a payload from
/// another version is told apart from a broken one. A copy sent more than a day after
/// `now`, or with more than `mostTimes` times, is invalid.
func decodeShared(_ body: Data, now: Int) -> Result<SharedSchedule, Unreadable> {
  guard let json = try? JSONSerialization.jsonObject(with: body) as? [String: Any],
    let version = json["v"]
  else { return .failure(.invalid) }
  guard let number = version as? NSNumber, CFGetTypeID(number) != CFBooleanGetTypeID(),
    number.doubleValue == Double(sharedVersion)
  else { return .failure(.version) }
  guard let payload = try? JSONDecoder().decode(Payload.self, from: body),
    payload.sentAt <= now + aheadAllowedSeconds,
    let profile = payload.profile.profile, profile.entries.count <= mostTimes
  else { return .failure(.invalid) }
  return .success(SharedSchedule(profile: profile, sentAt: payload.sentAt))
}
