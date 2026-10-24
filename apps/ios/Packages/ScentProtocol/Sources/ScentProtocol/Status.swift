// Whether a diffuser is reachable, and whether a reading is still current. Times are
// seconds since 1970. Ported from `packages/scent-core/src/status.rs`.

/// How often to ping each diffuser.
public let pingEverySeconds = 30
/// How long after a ping to wait for the answer before calling it offline.
public let answerWithinSeconds = 15
/// How long after hearing from a diffuser to still call it online.
public let onlineForSeconds = 75
/// How old a reading may get before the app greys it out.
public let staleAfterSeconds = 300

/// What we can say about a diffuser's connection right now.
public enum Online: Sendable, Hashable {
  /// Heard from within the last `onlineForSeconds`.
  case online
  /// Pinged within the last `answerWithinSeconds` and no answer yet.
  case waiting
  /// Pinged longer ago than `answerWithinSeconds` and nothing heard since.
  case offline
  /// No recent evidence either way.
  case unknown
}

/// `pingAt` is when we last pinged, `heardAt` when we last heard anything from the
/// diffuser. Hearing something at or after the last ping (or with no ping at all)
/// settles it: online if recent, unknown if not. Otherwise a ping is still outstanding:
/// waiting for `answerWithinSeconds`, then offline.
public func onlineStatus(pingAt: Int?, heardAt: Int?, now: Int) -> Online {
  if let heard = heardAt, pingAt.map({ heard >= $0 }) ?? true {
    return saturatingSubtract(now, heard) <= onlineForSeconds ? .online : .unknown
  }
  if let ping = pingAt {
    return saturatingSubtract(now, ping) <= answerWithinSeconds ? .waiting : .offline
  }
  return .unknown
}

/// A reading taken at `at` is stale once more than `staleAfterSeconds` have passed.
public func isStale(at: Int, now: Int) -> Bool {
  saturatingSubtract(now, at) > staleAfterSeconds
}

/// `a - b`, stopping at the ends instead of overflowing.
func saturatingSubtract(_ a: Int, _ b: Int) -> Int {
  let (difference, overflow) = a.subtractingReportingOverflow(b)
  return overflow ? (b < 0 ? Int.max : Int.min) : difference
}
