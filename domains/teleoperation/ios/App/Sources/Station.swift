import Foundation

/// The Rust station, as SwiftUI sees it: polled a few times a second.
final class StationModel: ObservableObject {
    @Published var link = "lost"
    @Published var vehicle = "disarmed"
    @Published var mode = "----"
    @Published var home = "not set"
    @Published var armButton = "ARM"

    private let station: T1SStation
    private var timer: Timer?

    init() {
        let env = ProcessInfo.processInfo.environment
        let address = env["T1S_ADDRESS"] ?? "tcpout:127.0.0.1:5760"
        let port = UInt16(env["T1S_CONTROL_PORT"] ?? "") ?? 0
        station = t1s_start(address, port)!
        timer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in self?.poll() }
    }

    private func observe(_ key: String) -> String? {
        guard let raw = t1s_observe(station, key) else { return nil }
        defer { t1s_string_free(raw) }
        return String(cString: raw)
    }

    private func poll() {
        link = observe("link") ?? link
        vehicle = observe("vehicle") ?? vehicle
        mode = observe("autopilot mode") ?? mode
        home = observe("home position") ?? home
        armButton = observe("arm button") ?? armButton
    }

    func press(_ id: String) {
        t1s_press(station, id)
        poll()
    }
}
