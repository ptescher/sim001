import XCTest

/// The touch half of the iOS driver. The Rust driver (domains/teleoperation/ios_driver) listens on
/// T1S_BRIDGE; this test connects, then does what it is told, one line at a time:
///
///     launch            start the app fresh            -> ok
///     tap <id>          touch the element with this id -> ok | err <why>
///     screenshot        the screen                      -> ok <base64 png>
///     quit              end the test                    -> ok
final class Bridge: XCTestCase {
    func testBridge() throws {
        continueAfterFailure = false
        let env = ProcessInfo.processInfo.environment
        guard let target = env["T1S_BRIDGE"], let colon = target.lastIndex(of: ":"),
              let port = UInt16(target[target.index(after: colon)...])
        else { throw XCTSkip("Run by the iOS driver: T1S_BRIDGE=host:port says where it listens") }
        let line = try Line(host: String(target[..<colon]), port: port)
        let controlPort = env["T1S_CONTROL_PORT"] ?? "5790"
        let address = env["T1S_ADDRESS"] ?? "tcpout:127.0.0.1:5760"

        let app = XCUIApplication()
        while let request = line.read() {
            let (verb, rest) = request.split(separator: " ", maxSplits: 1).map(String.init).splitHead()
            switch verb {
            case "launch":
                app.terminate()
                app.launchEnvironment = ["T1S_CONTROL_PORT": controlPort, "T1S_ADDRESS": address]
                app.launch()
                line.write("ok")
            case "tap":
                let element = app.descendants(matching: .any)[rest].firstMatch
                if element.waitForExistence(timeout: 5) {
                    element.tap()
                    line.write("ok")
                } else {
                    line.write("err nothing on the screen is called \(rest)")
                }
            case "screenshot":
                line.write("ok " + XCUIScreen.main.screenshot().pngRepresentation.base64EncodedString())
            case "quit":
                app.terminate()
                line.write("ok")
                return
            default:
                line.write("err unknown request \(request)")
            }
        }
    }
}

private extension Array where Element == String {
    func splitHead() -> (String, String) { (first ?? "", count > 1 ? self[1] : "") }
}

/// A TCP connection read and written a line at a time.
final class Line {
    private let fd: Int32
    private var pending = Data()

    init(host: String, port: UInt16) throws {
        fd = socket(AF_INET, SOCK_STREAM, 0)
        var addr = sockaddr_in()
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = port.bigEndian
        addr.sin_addr.s_addr = inet_addr(host)
        let ok = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
        }
        if ok != 0 { throw NSError(domain: "bridge", code: Int(errno), userInfo: [NSLocalizedDescriptionKey: "can't reach \(host):\(port)"]) }
    }

    func read() -> String? {
        while true {
            if let nl = pending.firstIndex(of: 10) {
                let text = String(decoding: pending[..<nl], as: UTF8.self)
                pending.removeSubrange(...nl)
                return text
            }
            var buf = [UInt8](repeating: 0, count: 65536)
            let n = Darwin.read(fd, &buf, buf.count)
            if n <= 0 { return nil }
            pending.append(contentsOf: buf[..<n])
        }
    }

    func write(_ text: String) {
        let bytes = Array((text + "\n").utf8)
        var sent = 0
        while sent < bytes.count {
            let n = Darwin.write(fd, bytes[sent...].withUnsafeBufferPointer { $0.baseAddress! }, bytes.count - sent)
            if n <= 0 { return }
            sent += n
        }
    }
}
