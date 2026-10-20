import SwiftUI

@main
struct GroundStationApp: App {
    @StateObject private var station = StationModel()

    var body: some Scene {
        WindowGroup {
            CommandScreen(station: station).preferredColorScheme(.dark)
        }
    }
}
