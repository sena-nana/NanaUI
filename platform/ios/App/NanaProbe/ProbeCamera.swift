import AVFoundation
import UIKit

/// Front camera at 640×480, as NanaTracking captures: a preview layer for
/// the native path and BGRA sample buffers for the CPU path.
final class ProbeCamera: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    let previewLayer = AVCaptureVideoPreviewLayer()
    /// Called on the main thread with each frame while `wantsFrames`.
    var onFrame: ((CVPixelBuffer) -> Void)?
    var wantsFrames: Bool {
        get { frames.withLock { $0 } }
        set { frames.withLock { $0 = newValue } }
    }

    private let session = AVCaptureSession()
    private let queue = DispatchQueue(label: "dev.nana.probe.camera")
    private let frames = Locked(false)

    override init() {
        super.init()
        previewLayer.session = session
        previewLayer.videoGravity = .resizeAspectFill
        // Black until the first frame, as the system camera shows.
        previewLayer.backgroundColor = UIColor.black.cgColor
    }

    func start() {
        AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
            guard granted, let self else { return }
            self.queue.async { self.configure() }
        }
    }

    private func configure() {
        session.beginConfiguration()
        session.sessionPreset = .vga640x480
        guard
            let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .front),
            let input = try? AVCaptureDeviceInput(device: device),
            session.canAddInput(input)
        else {
            session.commitConfiguration()
            return
        }
        session.addInput(input)
        let output = AVCaptureVideoDataOutput()
        output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA]
        output.alwaysDiscardsLateVideoFrames = true
        output.setSampleBufferDelegate(self, queue: queue)
        if session.canAddOutput(output) { session.addOutput(output) }
        session.commitConfiguration()
        session.startRunning()
    }

    func captureOutput(
        _ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection
    ) {
        guard wantsFrames, let buffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        DispatchQueue.main.async { [weak self] in self?.onFrame?(buffer) }
    }
}

/// A value behind a lock, read from the camera queue and written on main.
final class Locked<Value>: @unchecked Sendable {
    private var value: Value
    private let lock = NSLock()

    init(_ value: Value) { self.value = value }

    func withLock<R>(_ body: (inout Value) -> R) -> R {
        lock.lock()
        defer { lock.unlock() }
        return body(&value)
    }
}
