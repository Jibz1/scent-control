import Foundation
import ScentProtocol

/// Pretends to be the diffusers, for previews and screenshots: every action plays out on
/// sample data with believable delays, and nothing is sent anywhere.
@MainActor
public final class SampleEngine: Engine {
  private weak var model: AppModel?
  private var nextLogID = 100

  public init() {}

  public func attach(_ model: AppModel) {
    self.model = model
  }

  public func submit(_ action: Action) {
    guard let model else { return }
    switch action {
    // The route is worked out before the change: the report runs inside `model.update`,
    // where reading the diffusers again would overlap the write.
    case .setPower(let id, let on):
      let via = route(id, model)
      change(id, in: model, pending: Pending(power: on, stage: .sending(via))) {
        $0.power = Reading(on, at: .now, via: via ?? .wifi)
      }
      logSent(on ? "Turned on" : "Turned off", id, model)
    case .setLevel(let id, let level):
      let via = route(id, model)
      change(id, in: model, pending: Pending(level: level, stage: .sending(via))) {
        $0.level = Reading(level, at: .now, via: via ?? .wifi)
      }
      logSent("Set intensity to \(Self.levelName(level))", id, model)
    case .readStatus(let id):
      model.update(id) { $0.lastReport = .now }
    case .setSendOver(let id, let sendOver):
      model.update(id) { $0.sendOver = sendOver }
    case .scan:
      model.scanning = true
      later(2.4) {
        model.scanning = false
        if model.found.isEmpty {
          model.found = [Samples.foundAir2]
        }
      }
    case .sendSchedule(let id, let profile):
      model.update(id) {
        $0.schedule = profile
        $0.scheduleSentAt = .now
      }
      logSent("Sent a schedule with \(profile.entries.count) times", id, model)
    case .copySchedule(let from, let targets):
      guard let profile = model.diffuser(from)?.schedule else { return }
      for (index, target) in targets.enumerated() {
        model.copies[target] = .sending
        later(0.9 + 0.6 * Double(index)) {
          model.update(target) {
            $0.schedule = profile
            $0.scheduleSentAt = .now
          }
          model.copies[target] = .copied
        }
      }
    case .rename(let id, let name, let room):
      model.update(id) {
        $0.name = name
        $0.room = room
      }
    case .add(let found, let name, let room, _, _):
      playAdd(found, name: name, room: room, model: model)
    case .setAppearance(let appearance):
      model.appearance = appearance
    case .appActive:
      break
    case .saveBroker(let settings, _):
      model.brokerSettings = settings
      model.broker = .connecting
      later(1.2) { model.broker = .connected }
    }
  }

  private func route(_ id: String, _ model: AppModel) -> Via? {
    guard let diffuser = model.diffuser(id) else { return nil }
    switch diffuser.sendOver {
    case .auto: return diffuser.bluetoothLinked ? .bluetooth : .wifi
    case .wifi: return .wifi
    case .bluetooth: return .bluetooth
    }
  }

  /// Shows `pending`, then over WiFi "waiting for the diffuser", then the report.
  private func change(
    _ id: String, in model: AppModel, pending: Pending,
    report: @escaping @MainActor (inout DiffuserState) -> Void
  ) {
    model.update(id) {
      $0.pending = pending
      $0.failure = nil
    }
    let overWifi = pending.stage == .sending(.wifi)
    later(0.6) {
      if overWifi {
        model.update(id) { $0.pending?.stage = .waitingForDiffuser }
      }
      self.later(overWifi ? 1.4 : 0.2) {
        model.update(id) {
          report(&$0)
          $0.pending = nil
          $0.lastReport = .now
        }
      }
    }
  }

  private func playAdd(_ found: UUID, name: String?, room: String?, model: AppModel) {
    guard let diffuser = model.found.first(where: { $0.id == found }) else { return }
    var progress = AddProgress(
      deviceID: nil, kind: diffuser.kind, variant: diffuser.variant, name: name, room: room,
      steps: [
        (.read, .working), (.moveToBroker, .waiting), (.restart, .waiting),
        (.waitForConnect, .waiting),
      ])
    model.adds[found] = progress
    let wait: TimeInterval = 8
    later(1.0) {
      progress.deviceID = "TESTDEVICE04"
      progress.steps[0].state = .done
      progress.steps[1].state = .countdown(until: .now + wait, total: wait)
      model.adds[found] = progress
      self.later(wait) {
        progress.steps[1].state = .done
        progress.steps[2].state = .working
        model.adds[found] = progress
        self.later(1.2) {
          progress.steps[2].state = .done
          progress.steps[3].state = .working
          model.adds[found] = progress
          self.later(2.2) {
            progress.steps[3].state = .done
            progress.finished = true
            model.adds[found] = progress
            model.found.removeAll { $0.id == found }
            model.diffusers.append(
              DiffuserState(
                id: "TESTDEVICE04", name: name, room: room, kind: diffuser.kind,
                variant: diffuser.variant, firmware: "2.30", reachability: .answering(.wifi),
                lastReport: .now, bluetoothLinked: true))
          }
        }
      }
    }
  }

  private func logSent(_ what: String, _ id: String, _ model: AppModel) {
    nextLogID += 1
    let title = model.diffuser(id)?.title ?? id
    model.log.insert(
      LogLine(id: nextLogID, at: .now, mark: .sent, sentence: "\(what) on \(title)"), at: 0)
  }

  private func later(_ seconds: Double, _ work: @escaping @MainActor () -> Void) {
    Task { @MainActor in
      try? await Task.sleep(for: .seconds(seconds))
      work()
    }
  }

  static func levelName(_ level: Level) -> String {
    ["Low", "Medium", "High"][level.value - 1]
  }
}
