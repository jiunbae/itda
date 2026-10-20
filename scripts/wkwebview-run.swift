// Open URLs one after another in an offscreen WKWebView (the macOS app's engine) and wait
// for each page to set document.title = "done". Used by the H.264 end-to-end test.
//   swiftc -O scripts/wkwebview-run.swift -o /tmp/wkrun && /tmp/wkrun URL...
import Cocoa
import WebKit

final class Runner: NSObject, WKNavigationDelegate {
    var urls: [URL]
    let view = WKWebView(frame: NSRect(x: 0, y: 0, width: 1920, height: 1080))
    init(_ urls: [URL]) { self.urls = urls; super.init(); view.navigationDelegate = self }
    func next() {
        guard !urls.isEmpty else { exit(0) }
        view.load(URLRequest(url: urls.removeFirst()))
        poll(deadline: Date().addingTimeInterval(30))
    }
    func poll(deadline: Date) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
            self.view.evaluateJavaScript("document.title") { r, _ in
                if (r as? String) == "done" { return self.next() }
                if Date() > deadline { print("timeout: \(self.view.url?.absoluteString ?? "")"); exit(1) }
                self.poll(deadline: deadline)
            }
        }
    }
}
let app = NSApplication.shared
// Keep the view in an offscreen window so WebKit renders and decodes normally.
let window = NSWindow(contentRect: NSRect(x: -10000, y: -10000, width: 1920, height: 1080), styleMask: [.borderless], backing: .buffered, defer: false)
let runner = Runner(CommandLine.arguments.dropFirst().compactMap(URL.init(string:)))
window.contentView = runner.view
window.orderBack(nil)
runner.next()
app.run()
