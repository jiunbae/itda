// Offscreen VideoToolbox benchmark. No capture, input, permission or user-file access.
// swiftc -O scripts/hp-quality-bench.swift -o /tmp/itda-hp-quality-bench
// bench <output-dir> <point-width> <point-height> <scale:1|2> <bits-per-pixel-second> <frames>
import Foundation
import CoreGraphics
import CoreText
import CoreVideo
import CoreMedia
import VideoToolbox
let a = CommandLine.arguments
precondition(a.count == 7, "output-dir width height scale bpp-seconds frames")
let dir = a[1], pw = Int(a[2])!, ph = Int(a[3])!, scale = Int(a[4])!, rate = Int(a[5])!, count = Int(a[6])!
let w = pw * scale, h = ph * scale
precondition(w > 0 && h > 0 && w <= 4096 && h <= 4096 && count >= 30 && count <= 600 && (1...2).contains(scale) && (1...30).contains(rate))
try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
func monoMs() -> Double { Double(DispatchTime.now().uptimeNanoseconds) / 1e6 }
var buffers = [CVPixelBuffer]()
let font = CTFontCreateWithName("Menlo" as CFString, 12, nil)
for phase in 0..<8 {
    var raw: CVPixelBuffer?
    precondition(CVPixelBufferCreate(nil, w, h, kCVPixelFormatType_32BGRA, [kCVPixelBufferIOSurfacePropertiesKey: [:]] as CFDictionary, &raw) == 0)
    let p = raw!
    CVPixelBufferLockBaseAddress(p, [])
    let ctx = CGContext(data: CVPixelBufferGetBaseAddress(p), width: w, height: h, bitsPerComponent: 8, bytesPerRow: CVPixelBufferGetBytesPerRow(p), space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)!
    ctx.scaleBy(x: CGFloat(scale), y: CGFloat(scale))
    ctx.setFillColor(CGColor(gray: 1, alpha: 1)); ctx.fill(CGRect(x: 0, y: 0, width: pw, height: ph))
    for row in 0..<(ph / 20 + 1) {
        let text = String(format: "%03d  File transfer · 정보공개 결정통지서.pdf  let pixel = frame[x + y * stride];  AaBb 0123456789", row + phase)
        let ink = row % 7 == 0 ? CGColor(red: 0.08, green: 0.33, blue: 0.85, alpha: 1) : CGColor(gray: 0.12, alpha: 1)
        let attrs: [NSAttributedString.Key: Any] = [NSAttributedString.Key(kCTFontAttributeName as String): font, NSAttributedString.Key(kCTForegroundColorAttributeName as String): ink]
        let line = CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: attrs))
        ctx.textPosition = CGPoint(x: 20, y: ph - 25 - row * 20 + phase * 2)
        CTLineDraw(line, ctx)
    }
    ctx.setFillColor(CGColor(red: 0.15, green: 0.45, blue: 0.85, alpha: 1)); ctx.fill(CGRect(x: 50 + phase * 20, y: 12, width: 160, height: 45))
    CVPixelBufferUnlockBaseAddress(p, []); buffers.append(p)
}
final class Output { let lock = NSLock(); var data = Data(); var frames = 0; var last = 0; var bytes = 0; var errors = [Int32]() }
let out = Output()
var session: VTCompressionSession?
let spec = [kVTVideoEncoderSpecification_EnableLowLatencyRateControl: true] as CFDictionary
precondition(VTCompressionSessionCreate(allocator: nil, width: Int32(w), height: Int32(h), codecType: kCMVideoCodecType_H264, encoderSpecification: spec, imageBufferAttributes: nil, compressedDataAllocator: nil, outputCallback: { ref, _, status, _, sample in
    let o = Unmanaged<Output>.fromOpaque(ref!).takeUnretainedValue()
    o.lock.lock(); defer { o.lock.unlock() }
    guard status == 0, let s = sample, let block = CMSampleBufferGetDataBuffer(s) else { if status != 0 { o.errors.append(status) }; return }
    let format = CMSampleBufferGetFormatDescription(s)!
    let attachments = CMSampleBufferGetSampleAttachmentsArray(s, createIfNecessary: false) as? [[CFString: Any]]
    let key = attachments?.first?[kCMSampleAttachmentKey_NotSync] as? Bool != true
    if key {
        for i in 0..<2 {
            var p: UnsafePointer<UInt8>?, size = 0
            precondition(CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, parameterSetIndex: i, parameterSetPointerOut: &p, parameterSetSizeOut: &size, parameterSetCountOut: nil, nalUnitHeaderLengthOut: nil) == 0)
            o.data.append(contentsOf: [0,0,0,1]); o.data.append(p!, count: size)
        }
    }
    let len = CMBlockBufferGetDataLength(block)
    var bytes = [UInt8](repeating: 0, count: len)
    precondition(CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: len, destination: &bytes) == 0)
    var i = 0
    while i + 4 <= len {
        let n = Int(bytes[i]) << 24 | Int(bytes[i+1]) << 16 | Int(bytes[i+2]) << 8 | Int(bytes[i+3]); i += 4
        precondition(i + n <= len)
        o.data.append(contentsOf: [0,0,0,1]); o.data.append(contentsOf: bytes[i..<i+n]); i += n
    }
    o.frames += 1; o.last = Int(CMSampleBufferGetPresentationTimeStamp(s).value); o.bytes += len
}, refcon: Unmanaged.passUnretained(out).toOpaque(), compressionSessionOut: &session) == 0)
let s = session!, bitrate = min(40_000_000, max(4_000_000, w * h * rate))
for (key, val) in [(kVTCompressionPropertyKey_RealTime, true as CFBoolean), (kVTCompressionPropertyKey_AllowFrameReordering, false as CFBoolean)] { precondition(VTSessionSetProperty(s, key: key, value: val) == 0) }
precondition(VTSessionSetProperty(s, key: kVTCompressionPropertyKey_ProfileLevel, value: kVTProfileLevel_H264_High_AutoLevel) == 0)
precondition(VTSessionSetProperty(s, key: kVTCompressionPropertyKey_AverageBitRate, value: NSNumber(value: bitrate)) == 0)
precondition(VTSessionSetProperty(s, key: kVTCompressionPropertyKey_MaxKeyFrameInterval, value: NSNumber(value: Int32.max)) == 0)
precondition(VTSessionSetProperty(s, key: kVTCompressionPropertyKey_ExpectedFrameRate, value: NSNumber(value: 60)) == 0)
precondition(VTCompressionSessionPrepareToEncodeFrames(s) == 0)
var times = [Double]()
let start = monoMs()
for i in 0..<count {
    let due = start + Double(i) * 1000 / 60
    if due > monoMs() { Thread.sleep(forTimeInterval: (due - monoMs()) / 1000) }
    let t = monoMs(), pts = CMTime(value: Int64(i), timescale: 60)
    precondition(VTCompressionSessionEncodeFrame(s, imageBuffer: buffers[i % 8], presentationTimeStamp: pts, duration: .invalid, frameProperties: i == 0 ? [kVTEncodeFrameOptionKey_ForceKeyFrame: true] as CFDictionary : nil, sourceFrameRefcon: nil, infoFlagsOut: nil) == 0)
    precondition(VTCompressionSessionCompleteFrames(s, untilPresentationTimeStamp: pts) == 0)
    if i >= 20 { times.append(monoMs() - t) }
}
let elapsed = monoMs() - start
var hardwareRaw: UnsafeRawPointer?
let hardwareStatus = VTSessionCopyProperty(s, key: kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder, allocator: nil, valueOut: &hardwareRaw)
let hardwareValue = hardwareRaw.map { Unmanaged<AnyObject>.fromOpaque($0).takeRetainedValue() }
VTCompressionSessionInvalidate(s)
try out.data.write(to: URL(fileURLWithPath: dir + "/encoded.h264"))
let ref = buffers[out.last % 8]; CVPixelBufferLockBaseAddress(ref, .readOnly)
try Data(bytes: CVPixelBufferGetBaseAddress(ref)!, count: CVPixelBufferGetBytesPerRow(ref) * h).write(to: URL(fileURLWithPath: dir + "/reference.bgra"))
let bpr = CVPixelBufferGetBytesPerRow(ref); CVPixelBufferUnlockBaseAddress(ref, .readOnly)
times.sort()
let result: [String: Any] = ["width":w,"height":h,"scale":scale,"rate":rate,"target_bps":bitrate,"actual_mbps":Double(out.data.count)*8/(elapsed*1000),"frames":out.frames,"submitted":count,"dropped":count-out.frames,"elapsed_ms":elapsed,"encode_p50_ms":times[times.count/2],"encode_p95_ms":times[Int(Double(times.count-1)*0.95)],"last_source_index":out.last,"reference_stride":bpr,"hardware":hardwareValue as? Bool as Any? ?? NSNull(),"hardware_property_status":hardwareStatus,"errors":out.errors]
let json = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys]); try json.write(to: URL(fileURLWithPath: dir + "/result.json")); print(String(data:json,encoding:.utf8)!)
