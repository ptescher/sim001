import SwiftUI

/// The vehicle command screen: link, mode and arming, then ARM/DISARM and a button per mode.
struct CommandScreen: View {
    @ObservedObject var station: StationModel

    // Modes the T1-S VTOL flies (the console shows the same buttons).
    private let modes = ["MANUAL", "FBWA", "CRUISE", "AUTO", "RTL", "LOITER", "QSTABILIZE", "QHOVER", "QLOITER", "QLAND", "QRTL"]

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    annunciator("LINK", station.link == "connected" ? .green : .red)
                    annunciator("ARMED", station.vehicle == "armed" ? .yellow : .gray)
                    annunciator("HOME", station.home == "set" ? .green : .gray)
                    Spacer()
                    Text(station.mode)
                        .font(.system(.title2, design: .monospaced).bold())
                        .accessibilityIdentifier("mode-readout")
                }
                Button(station.armButton) {
                    station.press(station.vehicle == "armed" ? "disarm" : "arm")
                }
                .buttonStyle(CommandButton(danger: true))
                .accessibilityIdentifier(station.vehicle == "armed" ? "disarm" : "arm")

                LazyVGrid(columns: [GridItem(.adaptive(minimum: 120), spacing: 8)], spacing: 8) {
                    ForEach(modes, id: \.self) { m in
                        Button(m) { station.press("mode-\(m)") }
                            .buttonStyle(CommandButton(lit: station.mode == m))
                            .accessibilityIdentifier("mode-\(m)")
                    }
                }
            }
            .padding(16)
        }
    }

    private func annunciator(_ text: String, _ color: Color) -> some View {
        Text(text)
            .font(.system(.caption, design: .monospaced).bold())
            .padding(.horizontal, 8).padding(.vertical, 4)
            .background(color.opacity(0.85), in: RoundedRectangle(cornerRadius: 4))
    }
}

struct CommandButton: ButtonStyle {
    var danger = false
    var lit = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(.body, design: .monospaced).bold())
            .frame(maxWidth: .infinity, minHeight: 48)
            .background(lit ? Color.green.opacity(0.6) : danger ? Color.red.opacity(0.5) : Color.gray.opacity(0.3),
                        in: RoundedRectangle(cornerRadius: 6))
            .opacity(configuration.isPressed ? 0.6 : 1)
    }
}
