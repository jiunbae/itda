import XCTest
import UIKit

/// Compile-only scaffold. These bodies are proposals, not completed native tests.
/// There is deliberately no environment variable capable of enabling runtime.
@MainActor
final class ItdaMobileSafariTests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
        throw XCTSkip("RUNTIME BLOCKED: per-new-device/process pasteboard isolation is unresolved. Independent review and a later code change are required before any Simulator boot, Safari launch, or UI input.")
    }

    private struct Fixture {
        let base: URL
        let controlToken: String
    }

    private func fixture() throws -> Fixture {
        let env = ProcessInfo.processInfo.environment
        guard let raw = env["ITDA_QA_FIXTURE_URL"], let url = URL(string: raw),
              url.scheme == "http", url.host == "127.0.0.1", url.port != nil,
              url.user == nil, url.password == nil, url.query == nil, url.fragment == nil,
              url.path.isEmpty || url.path == "/",
              let token = env["ITDA_QA_CONTROL_TOKEN"], token.count == 48,
              token.allSatisfy({ $0.isHexDigit }) else {
            throw XCTSkip("An owned loopback fixture URL and its private control token are required. Never use personal Itda or a real device gateway.")
        }
        return Fixture(base: url, controlToken: token)
    }

    private func control(_ path: String, method: String = "GET") async throws -> [String: Any] {
        let f = try fixture()
        var req = URLRequest(url: f.base.appendingPathComponent("__qa/" + path))
        req.httpMethod = method
        req.timeoutInterval = 5
        req.setValue(f.controlToken, forHTTPHeaderField: "X-Itda-QA")
        // Test-fixture HTTP only. If this runner's ATS rejects it, stop; do not
        // alter host Safari, install certificates, or weaken product security.
        let session = URLSession(configuration: .ephemeral)
        defer { session.invalidateAndCancel() }
        let (data, response) = try await session.data(for: req)
        guard (response as? HTTPURLResponse)?.statusCode == 200,
              let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw NSError(domain: "ItdaMobileQA", code: 1, userInfo: [NSLocalizedDescriptionKey: "Owned QA control endpoint did not return the expected response"])
        }
        return object
    }

    private func browser() throws -> XCUIApplication {
        let safari = XCUIApplication(bundleIdentifier: "com.apple.mobilesafari")
        safari.open(try fixture().base)
        XCTAssertTrue(safari.webViews.firstMatch.waitForExistence(timeout: 10))
        return safari
    }

    private func evidence(_ name: String, safari: XCUIApplication) {
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = name + "-screen"; screenshot.lifetime = .keepAlways; add(screenshot)
        let tree = XCTAttachment(string: safari.debugDescription)
        tree.name = name + "-native-accessibility"; tree.lifetime = .keepAlways; add(tree)
    }

    private func receiverEvidence(_ name: String, stats: [String: Any]) throws {
        let bytes = try JSONSerialization.data(withJSONObject: stats, options: [.sortedKeys, .prettyPrinted])
        let attachment = XCTAttachment(data: bytes, uniformTypeIdentifier: "public.json")
        attachment.name = name + "-receiver"; attachment.lifetime = .keepAlways; add(attachment)
        let peers = stats["peers"] as? [[String: Any]] ?? []
        XCTAssertTrue(peers.allSatisfy { ($0["errors"] as? [String] ?? []).isEmpty }, "RFB fixture reported an unsupported/unsafe message")
        XCTAssertTrue(peers.allSatisfy { ($0["cutTextMessages"] as? Int) == 0 }, "Clipboard is outside this test's scope")
    }

    private func keyboard(_ safari: XCUIApplication) throws -> XCUIElement {
        let keyboard = safari.keyboards.firstMatch
        guard keyboard.waitForExistence(timeout: 5), !keyboard.frame.isEmpty else {
            evidence("BLOCKED-software-keyboard-absent", safari: safari)
            throw NSError(domain: "ItdaMobileQA", code: 2, userInfo: [NSLocalizedDescriptionKey: "Visible software keyboard is required. Do not substitute typeText, hardware events, or host keyboard preferences."])
        }
        return keyboard
    }

    private func tapKey(_ label: String, keyboard: XCUIElement) throws {
        // Exact native AX key must be observed, unique, and hittable. No coordinate
        // guessing and no sendKeys/typeText fallback if the keyboard differs.
        let matches = keyboard.keys.matching(NSPredicate(format: "label == %@ OR identifier == %@", label, label))
        guard matches.count == 1, matches.firstMatch.isHittable else {
            throw NSError(domain: "ItdaMobileQA", code: 3, userInfo: [NSLocalizedDescriptionKey: "Native keyboard key '\(label)' is missing or ambiguous; inspect the captured AX tree."])
        }
        matches.firstMatch.tap()
    }

    private func enterPIN(_ value: String, safari: XCUIApplication) async throws {
        let before = (try await control("stats"))["loginStatuses"] as? [Int] ?? []
        let pin = safari.webViews.secureTextFields["PIN"]
        XCTAssertTrue(pin.waitForExistence(timeout: 5)); pin.tap()
        let keys = try keyboard(safari)
        evidence("PIN-software-keyboard", safari: safari)
        // Clears selected or unselected old input through the actual keyboard.
        for _ in 0..<6 { try tapKey("delete", keyboard: keys) }
        for digit in value { try tapKey(String(digit), keyboard: keys) }
        let submit = safari.webViews.buttons["들어가기"]
        XCTAssertTrue(submit.isHittable, "Submit must remain reachable above the keyboard")
        submit.tap()
        // Pair every native submit with a new HTTP receiver event; existing PIN
        // guidance text alone is not proof that this submission was processed.
        for _ in 0..<50 {
            let after = (try await control("stats"))["loginStatuses"] as? [Int] ?? []
            if after.count == before.count + 1 { return }
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTFail("Native submit produced no new fixture HTTP response")
    }

    private func visibleText(_ text: String, safari: XCUIApplication, timeout: TimeInterval = 5) -> Bool {
        safari.webViews.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch.waitForExistence(timeout: timeout)
    }

    func testPINErrorRecoveryAndExpiredSession() async throws {
        _ = try await control("reset", method: "POST")
        let safari = try browser()
        try await enterPIN("111111", safari: safari)
        XCTAssertTrue(visibleText("PIN이 맞지 않습니다", safari: safari))
        XCTAssertTrue(visibleText("공유 페이지", safari: safari), "Wrong PIN must offer the recovery location")
        evidence("wrong-PIN-recovery", safari: safari)
        try await enterPIN("123456", safari: safari)
        XCTAssertTrue(safari.webViews.buttons["QA 화면 열기"].waitForExistence(timeout: 5))
        try receiverEvidence("PIN-recovery", stats: await control("stats"))
        _ = try await control("expire", method: "POST")
        safari.webViews.buttons["새로 고침"].tap()
        XCTAssertTrue(visibleText("PIN", safari: safari))
        XCTAssertTrue(visibleText("다시", safari: safari), "Expired session must explain why sign-in is needed")
        evidence("expired-session", safari: safari)
        try await enterPIN("123456", safari: safari)
        XCTAssertTrue(safari.webViews.buttons["QA 화면 열기"].waitForExistence(timeout: 5))
        try receiverEvidence("expired-session-recovery", stats: await control("stats"))
    }

    func testPINLockoutWaitAndRecovery() async throws {
        _ = try await control("reset", method: "POST")
        let safari = try browser()
        // Five failures arm the fixture lock; the next attempt gets 429,
        // matching src/gateway.rs. No clock override or production lockout change.
        for _ in 0..<6 {
            try await enterPIN("111111", safari: safari)
            XCTAssertTrue(visibleText("PIN", safari: safari))
        }
        XCTAssertTrue(visibleText("초 뒤", safari: safari))
        XCTAssertFalse(safari.webViews.buttons["들어가기"].isEnabled)
        evidence("PIN-locked", safari: safari)
        let before = try await control("stats")
        let ready = NSPredicate(format: "enabled == true")
        let wait = XCTNSPredicateExpectation(predicate: ready, object: safari.webViews.buttons["들어가기"])
        await fulfillment(of: [wait], timeout: 65)
        let after = try await control("stats")
        XCTAssertEqual(before["loginStatuses"] as? [Int], after["loginStatuses"] as? [Int], "Countdown must not retry PINs in the background")
        try await enterPIN("123456", safari: safari)
        XCTAssertTrue(safari.webViews.buttons["QA 화면 열기"].waitForExistence(timeout: 5))
        try receiverEvidence("PIN-lockout-recovery", stats: await control("stats"))
    }

    /// Locate fixture pixels before a native touch, rather than assuming a canvas
    /// position behind Safari chrome. If screenshots/AX coordinates do not agree,
    /// this fails closed and records the screen for a later reviewed adjustment.
    private func fixtureCenter(_ safari: XCUIApplication) throws -> CGPoint {
        let image = XCUIScreen.main.screenshot().image
        guard let cg = image.cgImage,
              abs(image.size.width - safari.frame.width) < 1,
              abs(image.size.height - safari.frame.height) < 1 else {
            throw NSError(domain: "ItdaMobileQA", code: 4, userInfo: [NSLocalizedDescriptionKey: "Screenshot to native screen geometry is unverified"])
        }
        let w = cg.width, h = cg.height, row = w * 4
        var bytes = [UInt8](repeating: 0, count: row * h)
        let drawn = bytes.withUnsafeMutableBytes { buffer -> Bool in
            guard let context = CGContext(data: buffer.baseAddress, width: w, height: h, bitsPerComponent: 8, bytesPerRow: row,
                                          space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
            context.draw(cg, in: CGRect(x: 0, y: 0, width: w, height: h))
            return true
        }
        guard drawn else {
            throw NSError(domain: "ItdaMobileQA", code: 5)
        }
        var minX = w, minY = h, maxX = -1, maxY = -1, count = 0
        for y in 0..<h { for x in 0..<w {
            let at = y * row + x * 4
            if abs(Int(bytes[at]) - 201) < 5 && abs(Int(bytes[at + 1]) - 37) < 5 && abs(Int(bytes[at + 2]) - 181) < 5 {
                minX = min(minX, x); maxX = max(maxX, x); minY = min(minY, y); maxY = max(maxY, y); count += 1
            }
        } }
        guard maxX - minX > 80, maxY - minY > 80,
              Double(count) > Double((maxX - minX + 1) * (maxY - minY + 1)) * 0.97 else {
            throw NSError(domain: "ItdaMobileQA", code: 6, userInfo: [NSLocalizedDescriptionKey: "The actual fictional framebuffer is not visibly identifiable; no touch sent"])
        }
        return CGPoint(x: Double(minX + maxX) / 2 / Double(w) * image.size.width,
                       y: Double(minY + maxY) / 2 / Double(h) * image.size.height)
    }

    private func waitForFixtureCenter(_ safari: XCUIApplication) async throws -> CGPoint {
        // Toolbar creation can precede the first framebuffer. Wait for actual
        // receiving pixels rather than tapping a loading overlay or fixed delay.
        for _ in 0..<20 {
            if let point = try? fixtureCenter(safari) { return point }
            try await Task.sleep(for: .milliseconds(250))
        }
        evidence("BLOCKED-framebuffer-not-identifiable", safari: safari)
        return try fixtureCenter(safari)
    }

    func testNativeTouchAndSoftwareKeyHaveReceiverEvidence() async throws {
        _ = try await control("reset", method: "POST")
        let safari = try browser()
        try await enterPIN("123456", safari: safari)
        let open = safari.webViews.buttons["QA 화면 열기"]
        XCTAssertTrue(open.waitForExistence(timeout: 5)); open.tap()
        XCTAssertTrue(safari.webViews.buttons["키보드"].waitForExistence(timeout: 10))
        evidence("connected-fictional-frame", safari: safari)
        let point = try await waitForFixtureCenter(safari)
        let before = try await control("stats")
        let oldPointerCount = (before["peers"] as? [[String: Any]])?.reduce(0) { $0 + (($1["pointer"] as? [[String: Any]])?.count ?? 0) } ?? 0
        safari.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: point.x, dy: point.y)).tap()
        safari.webViews.buttons["키보드"].tap()
        let keys = try keyboard(safari)
        evidence("remote-software-keyboard", safari: safari)
        try tapKey("a", keyboard: keys)
        // Poll only the bounded functional receiver, not timing/performance.
        var stats: [String: Any] = [:]
        for _ in 0..<30 {
            stats = try await control("stats")
            let events = (stats["peers"] as? [[String: Any]] ?? []).flatMap { $0["keys"] as? [[String: Any]] ?? [] }
            if events.contains(where: { ($0["key"] as? Int) == 97 && ($0["down"] as? Bool) == false }) { break }
            try await Task.sleep(for: .milliseconds(100))
        }
        let peers = stats["peers"] as? [[String: Any]] ?? []
        let pointer = peers.flatMap { $0["pointer"] as? [[String: Any]] ?? [] }
        let fresh = Array(pointer.dropFirst(oldPointerCount))
        XCTAssertTrue(fresh.contains { ($0["mask"] as? Int) == 1 && abs(($0["x"] as? Int ?? -1000) - 320) < 20 && abs(($0["y"] as? Int ?? -1000) - 240) < 20 })
        XCTAssertTrue(fresh.contains { ($0["mask"] as? Int) == 0 }, "Touch must release the remote button")
        let events = peers.flatMap { $0["keys"] as? [[String: Any]] ?? [] }
        XCTAssertTrue(events.contains { ($0["key"] as? Int) == 97 && ($0["down"] as? Bool) == true })
        XCTAssertTrue(events.contains { ($0["key"] as? Int) == 97 && ($0["down"] as? Bool) == false })
        try receiverEvidence("native-touch-and-ASCII-key", stats: stats)
        // This is not a physical tap, Korean IME, or real VNC-account test.
    }

    func testSecurityContextObservationOnly() throws {
        let safari = XCUIApplication(bundleIdentifier: "com.apple.mobilesafari")
        safari.open(try fixture().base.appendingPathComponent("qa-context.html"))
        XCTAssertTrue(visibleText("isSecureContext", safari: safari))
        evidence("loopback-capabilities-only-not-LAN-HTTPS", safari: safari)
    }

    func testClipboardAndCameraAreExplicitlyUnimplemented() throws {
        throw XCTSkip("No clipboard read/write, paste UI, QR camera, real LAN/HTTPS, trust bypass, or real-device input is implemented or allowed by this scaffold.")
    }
}
