import AVFoundation
import UIKit

/// The probe page: the NanaUI surface over a native camera preview layer
/// (hybrid layer ③). The page leaves its preview area transparent; this
/// controller lays the camera layer out to it after each presented frame.
/// With the page's CPU preview switch on, the layer hides and frames are
/// uploaded into the page's host texture instead, for comparison.
///
/// A transparent surface paints no page background; the root view paints
/// the theme's under the camera layer, so the page looks as it does opaque.
final class ProbeViewController: UIViewController {
    private let nanaView = NanaView(transparent: true)
    private let camera = ProbeCamera()
    private let metrics = ProbeMetrics()
    private var previewFrame = CGRect.zero
    private var background: [Float] = []

    override func loadView() {
        let root = UIView()
        root.backgroundColor = .black
        root.layer.addSublayer(camera.previewLayer)
        nanaView.frame = root.bounds
        nanaView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        root.addSubview(nanaView)
        view = root
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        nanaView.afterTick = { [weak self] in self?.layoutPreview() }
        nanaView.onTick = { [metrics] link, cpu in metrics.tick(link: link, cpu: cpu) }
        nanaView.onIdle = { [metrics] in metrics.idle() }
        camera.onFrame = { [weak self] buffer in self?.uploadPreview(buffer) }
        camera.start()
    }

    private func layoutPreview() {
        guard let host = nanaView.host else { return }
        var rgba = [Float](repeating: 0, count: 4)
        if nana_ios_host_background(host, &rgba), rgba != background {
            background = rgba
            view.backgroundColor = UIColor(
                red: CGFloat(rgba[0]), green: CGFloat(rgba[1]), blue: CGFloat(rgba[2]), alpha: CGFloat(rgba[3]))
        }
        var rect = NanaRect()
        let cpu = nana_probe_cpu_preview(host)
        camera.wantsFrames = cpu
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        camera.previewLayer.isHidden = cpu
        if nana_probe_preview_rect(host, &rect) {
            let frame = view.convert(
                CGRect(x: CGFloat(rect.x), y: CGFloat(rect.y), width: CGFloat(rect.width), height: CGFloat(rect.height)),
                from: nanaView)
            if frame != previewFrame {
                previewFrame = frame
                camera.previewLayer.frame = frame
                camera.previewLayer.cornerRadius = 12
                camera.previewLayer.masksToBounds = true
            }
        }
        CATransaction.commit()
    }

    /// The CPU path, on the main thread: copy the BGRA frame into the page's
    /// host texture.
    private func uploadPreview(_ buffer: CVPixelBuffer) {
        guard let host = nanaView.host else { return }
        CVPixelBufferLockBaseAddress(buffer, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddress(buffer) else { return }
        nana_probe_push_preview_bgra(
            host, base.assumingMemoryBound(to: UInt8.self),
            UInt32(CVPixelBufferGetWidth(buffer)), UInt32(CVPixelBufferGetHeight(buffer)),
            UInt32(CVPixelBufferGetBytesPerRow(buffer)))
        nanaView.requestFrame()
    }
}
