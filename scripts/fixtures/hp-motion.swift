// Temporary, nonactivating, click-through workload for before/after HP measurements.
// swiftc -O scripts/fixtures/hp-motion.swift -o /tmp/itda-hp-motion
// /tmp/itda-hp-motion [seconds, default 30, max 120]
// It closes itself, never injects input, never requests capture/accessibility permissions.
import AppKit

let seconds = min(120, max(1, CommandLine.arguments.dropFirst().first.flatMap(Double.init) ?? 30))
let origin = NSPoint(x: 40, y: 60)
let size = NSSize(width: 1280, height: 800)

final class Motion: NSView {
    var elapsed = 0.0
    override func draw(_ dirty: NSRect) {
        NSColor.white.setFill()
        bounds.fill()
        let offset = Int(elapsed * 500) % 4000
        for row in -1...40 {
            let y = CGFloat(row * 24 - offset % 24)
            (row % 9 == 0 ? NSColor.systemBlue : NSColor(calibratedWhite: 0.13, alpha: 1)).setFill()
            for column in 0..<105 where (column + (row + offset / 24) * 7) % 11 != 0 {
                NSRect(x: CGFloat(20 + column * 9), y: y, width: 6, height: 14).fill()
            }
        }
        NSColor.systemBlue.setFill()
        NSRect(x: CGFloat(Int(elapsed * 210) % 1100), y: 35, width: 150, height: 70).fill()
        // 24-bit epoch-ms modulo 2^24, preceded by white/black sentinel cells.
        // A receiver can estimate content age after accounting for measured clock skew.
        // This is a content timestamp, not proof of physical capture/presentation time.
        let stamp = UInt64(Date().timeIntervalSince1970 * 1000) & 0x00ff_ffff
        NSColor.gray.setFill()
        NSRect(x: 15, y: bounds.height - 45, width: 530, height: 30).fill()
        for i in 0..<26 {
            let white = i == 0 || (i >= 2 && (stamp & (1 << (i - 2))) != 0)
            (white ? NSColor.white : NSColor.black).setFill()
            NSRect(x: CGFloat(20 + i * 20), y: bounds.height - 40, width: 16, height: 20).fill()
        }
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let panel = NSPanel(contentRect: NSRect(origin: origin, size: size),
                    styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
panel.ignoresMouseEvents = true
panel.level = .floating
panel.title = "Itda temporary performance workload"
let view = Motion(frame: NSRect(origin: .zero, size: size))
panel.contentView = view
panel.orderFrontRegardless()
let screenHeight = NSScreen.main?.frame.height ?? 0
let metadata: [String: Any] = ["seconds": seconds, "width": size.width, "height": size.height,
    "barcode_x": origin.x + 28, "barcode_y": screenHeight - origin.y - size.height + 30,
    "barcode_step": 20, "barcode_bits": 24]
if let data = try? JSONSerialization.data(withJSONObject: metadata, options: [.sortedKeys]),
   let line = String(data: data, encoding: .utf8) { print(line); fflush(stdout) }
let started = Date()
let timer = Timer.scheduledTimer(withTimeInterval: 1.0 / 60, repeats: true) { _ in
    let elapsed = Date().timeIntervalSince(started)
    if elapsed >= seconds { panel.orderOut(nil); app.terminate(nil); return }
    view.elapsed = elapsed
    view.needsDisplay = true
}
app.run()
