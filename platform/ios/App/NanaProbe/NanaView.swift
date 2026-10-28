import QuartzCore
import UIKit

/// The UIKit side of one NanaUI document: a `CAMetalLayer` the Rust host
/// presents into, a display link that runs only while the document asks for
/// frames, touches lowered into the host, and the native text proxy.
final class NanaView: UIView {
    override class var layerClass: AnyClass { CAMetalLayer.self }

    private(set) var host: OpaquePointer?
    private var link: CADisplayLink?
    private var deadline: Timer?
    private var attached = true
    private let transparent: Bool
    let textProxy = NanaTextProxy()
    let metrics = ProbeMetrics()
    /// Runs after each presented tick: the view controller lays the native
    /// camera layer out to the page's preview area.
    var afterTick: (() -> Void)?

    private var metalLayer: CAMetalLayer { layer as! CAMetalLayer }

    init(transparent: Bool) {
        self.transparent = transparent
        super.init(frame: .zero)
        isMultipleTouchEnabled = true
        backgroundColor = .clear
        metalLayer.isOpaque = !transparent
        textProxy.nanaView = self
        addSubview(textProxy)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    deinit {
        link?.invalidate()
        if let host { nana_ios_host_free(host) }
    }

    // MARK: Geometry

    private var drawableSize: (UInt32, UInt32) {
        let scale = contentScaleFactor
        return (UInt32(max(1, bounds.width * scale)), UInt32(max(1, bounds.height * scale)))
    }

    private var safeArea: NanaSafeArea {
        let scale = Float(contentScaleFactor)
        let insets = safeAreaInsets
        return NanaSafeArea(
            top: Float(insets.top) * scale,
            right: Float(insets.right) * scale,
            bottom: Float(insets.bottom) * scale,
            left: Float(insets.left) * scale)
    }

    private var dark: Bool { traitCollection.userInterfaceStyle == .dark }

    /// Dynamic Type as a scale of the 17 pt body size.
    private var textScale: Float {
        Float(UIFontMetrics(forTextStyle: .body).scaledValue(for: 17, compatibleWith: traitCollection) / 17)
    }

    // MARK: Lifecycle

    override func didMoveToWindow() {
        super.didMoveToWindow()
        guard let window, host == nil else { return }
        contentScaleFactor = window.screen.scale
        metalLayer.contentsScale = contentScaleFactor
        createHost()
    }

    private func createHost() {
        let (width, height) = drawableSize
        let callbacks = NanaIosCallbacks(
            context: Unmanaged.passUnretained(self).toOpaque(),
            wake: nanaWake,
            text_input: nanaTextInput,
            pressed: nanaPressed)
        host = nana_ios_host_new(
            Unmanaged.passUnretained(metalLayer).toOpaque(), width, height,
            Float(contentScaleFactor), safeArea, dark, textScale, transparent, callbacks)
        guard host != nil else {
            ProbeLog.metrics.error("nana_ios_host_new failed")
            return
        }
        let link = CADisplayLink(target: DisplayLinkTarget(self), selector: #selector(DisplayLinkTarget.tick(_:)))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 120, preferred: 120)
        link.add(to: .main, forMode: .common)
        self.link = link
        registerForTraitChanges([UITraitUserInterfaceStyle.self, UITraitPreferredContentSizeCategory.self]) {
            (view: NanaView, _: UITraitCollection) in
            guard let host = view.host else { return }
            nana_ios_host_set_appearance(host, view.dark, view.textScale)
            view.requestFrame()
        }
        let center = NotificationCenter.default
        center.addObserver(self, selector: #selector(suspend), name: UIApplication.didEnterBackgroundNotification, object: nil)
        center.addObserver(self, selector: #selector(resume), name: UIApplication.willEnterForegroundNotification, object: nil)
    }

    /// Background: no GPU work, drop the surface, keep the document.
    @objc private func suspend() {
        guard let host, attached else { return }
        link?.isPaused = true
        deadline?.invalidate()
        nana_ios_host_detach(host)
        attached = false
    }

    @objc private func resume() {
        guard let host, !attached else { return }
        let (width, height) = drawableSize
        attached = nana_ios_host_attach(host, Unmanaged.passUnretained(metalLayer).toOpaque(), width, height)
        requestFrame()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard let host else { return }
        let (width, height) = drawableSize
        nana_ios_host_resize(host, width, height, Float(contentScaleFactor), safeArea)
        textProxy.editorMoved()
        requestFrame()
    }

    override func safeAreaInsetsDidChange() {
        super.safeAreaInsetsDidChange()
        setNeedsLayout()
    }

    // MARK: Frames

    /// Something changed: run the display link from the next vsync.
    func requestFrame() {
        guard attached else { return }
        deadline?.invalidate()
        deadline = nil
        link?.isPaused = false
    }

    fileprivate func tick(_ link: CADisplayLink) {
        guard let host else { return }
        let started = CACurrentMediaTime()
        var seconds = 0.0
        let demand = Int(nana_ios_host_tick(host, &seconds))
        metrics.tick(link: link, cpu: CACurrentMediaTime() - started)
        afterTick?()
        switch demand {
        case NANA_FRAME_NOW:
            break
        case NANA_FRAME_AT where seconds < link.duration:
            break
        case NANA_FRAME_AT:
            link.isPaused = true
            deadline = Timer.scheduledTimer(withTimeInterval: seconds, repeats: false) { [weak self] _ in
                self?.requestFrame()
            }
        default:
            link.isPaused = true
            metrics.idle()
        }
    }

    // MARK: Touch

    private func send(_ touches: Set<UITouch>, _ event: UIEvent?, phase: Int) {
        guard let host else { return }
        let scale = contentScaleFactor
        for touch in touches {
            let samples = phase == NANA_TOUCH_MOVE ? (event?.coalescedTouches(for: touch) ?? [touch]) : [touch]
            for sample in samples {
                let point = sample.location(in: self)
                _ = nana_ios_host_touch(
                    host, Int32(phase), Float(point.x * scale), Float(point.y * scale),
                    UInt64(UInt(bitPattern: ObjectIdentifier(touch).hashValue)))
            }
        }
        requestFrame()
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        send(touches, event, phase: NANA_TOUCH_DOWN)
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        send(touches, event, phase: NANA_TOUCH_MOVE)
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        send(touches, event, phase: NANA_TOUCH_UP)
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        send(touches, event, phase: NANA_TOUCH_CANCEL)
    }
}

/// Breaks the display link's strong reference to its target.
private final class DisplayLinkTarget: NSObject {
    weak var view: NanaView?

    init(_ view: NanaView) { self.view = view }

    @objc func tick(_ link: CADisplayLink) {
        guard let view else {
            link.invalidate()
            return
        }
        view.tick(link)
    }
}

// The host's callbacks run inside its input routing, or on a task thread:
// none of them calls back in synchronously.

private func view(_ context: UnsafeMutableRawPointer?) -> NanaView? {
    context.map { Unmanaged<NanaView>.fromOpaque($0).takeUnretainedValue() }
}

private let nanaWake: @convention(c) (UnsafeMutableRawPointer?) -> Void = { context in
    guard let view = view(context) else { return }
    DispatchQueue.main.async { view.requestFrame() }
}

private let nanaTextInput: @convention(c) (UnsafeMutableRawPointer?, Bool, NanaRect) -> Void = {
    context, active, _ in
    guard let view = view(context) else { return }
    DispatchQueue.main.async { view.textProxy.editorChanged(active: active) }
}

private let haptics = UIImpactFeedbackGenerator(style: .light)

private let nanaPressed: @convention(c) (UnsafeMutableRawPointer?) -> Void = { _ in
    DispatchQueue.main.async { haptics.impactOccurred() }
}
