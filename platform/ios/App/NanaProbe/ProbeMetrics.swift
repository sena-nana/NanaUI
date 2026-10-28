import Darwin
import QuartzCore
import os

enum ProbeLog {
    static let metrics = Logger(subsystem: "dev.nana.probe", category: "metrics")
    static let signposts = OSSignposter(subsystem: "dev.nana.probe", category: .pointsOfInterest)
}

/// Process start, for the cold-start-to-first-frame number.
enum ProbeClock {
    private(set) static var launched: CFTimeInterval = 0

    static func markLaunch() {
        launched = CACurrentMediaTime()
    }

    /// Seconds since the kernel started this process.
    static func sinceProcessStart() -> Double? {
        var info = kinfo_proc()
        var size = MemoryLayout<kinfo_proc>.stride
        var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, getpid()]
        guard sysctl(&mib, 4, &info, &size, nil, 0) == 0 else { return nil }
        let start = info.kp_proc.p_starttime
        var now = timeval()
        gettimeofday(&now, nil)
        return Double(now.tv_sec - start.tv_sec) + Double(now.tv_usec - start.tv_usec) / 1_000_000
    }
}

/// Frame pacing while the display link runs, logged every 240 ticks:
/// the interval between consecutive vsyncs the link fired on, and the CPU
/// time of each tick. Idle stretches (link paused) are not frames.
final class ProbeMetrics {
    private var lastTimestamp: CFTimeInterval?
    private var intervals: [Double] = []
    private var cpu: [Double] = []
    private var firstFrameLogged = false
    private var ticks = 0

    func tick(link: CADisplayLink, cpu seconds: Double) {
        ticks += 1
        ProbeLog.signposts.emitEvent("tick", "\(seconds * 1000, format: .fixed(precision: 2)) ms")
        if !firstFrameLogged {
            firstFrameLogged = true
            let sinceStart = ProbeClock.sinceProcessStart().map { String(format: "%.0f", $0 * 1000) } ?? "?"
            let sinceLaunch = (CACurrentMediaTime() - ProbeClock.launched) * 1000
            ProbeLog.metrics.notice(
                "first frame: \(sinceStart, privacy: .public) ms after process start, \(sinceLaunch, format: .fixed(precision: 0)) ms after launch")
        }
        if let last = lastTimestamp { intervals.append(link.timestamp - last) }
        lastTimestamp = link.timestamp
        cpu.append(seconds)
        if intervals.count >= 240 { report() }
    }

    /// The link paused: the next tick starts a new run.
    func idle() {
        lastTimestamp = nil
    }

    private func report() {
        func percentile(_ values: [Double], _ p: Double) -> Double {
            let sorted = values.sorted()
            return sorted[min(sorted.count - 1, Int(Double(sorted.count) * p))] * 1000
        }
        ProbeLog.metrics.notice(
            """
            frames=\(self.intervals.count) interval p50=\(percentile(self.intervals, 0.5), format: .fixed(precision: 2)) \
            p99=\(percentile(self.intervals, 0.99), format: .fixed(precision: 2)) ms; \
            tick cpu p50=\(percentile(self.cpu, 0.5), format: .fixed(precision: 2)) \
            p99=\(percentile(self.cpu, 0.99), format: .fixed(precision: 2)) ms
            """)
        intervals.removeAll(keepingCapacity: true)
        cpu.removeAll(keepingCapacity: true)
    }
}
