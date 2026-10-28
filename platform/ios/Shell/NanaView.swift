import QuartzCore
import UIKit
import os

private let shellLog = Logger(subsystem: Bundle.main.bundleIdentifier ?? "nana-ui", category: "shell")

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
    /// Runs after each tick: an app lays native layers out to the page here.
    var afterTick: (() -> Void)?
    /// Each display-link tick with the host's CPU time for it, for an app's
    /// own frame metrics.
    var onTick: ((CADisplayLink, CFTimeInterval) -> Void)?
    /// The document went idle and the display link paused.
    var onIdle: (() -> Void)?
    private var accessibilityCount = 0

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
            haptic: nanaHaptic)
        host = nana_ios_host_new(
            Unmanaged.passUnretained(metalLayer).toOpaque(), width, height,
            Float(contentScaleFactor), safeArea, dark, textScale, transparent, callbacks)
        guard host != nil else {
            shellLog.error("nana_ios_host_new failed")
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
        center.addObserver(self, selector: #selector(suspend), name: UIScene.didEnterBackgroundNotification, object: nil)
        center.addObserver(self, selector: #selector(resume), name: UIScene.willEnterForegroundNotification, object: nil)
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
        onTick?(link, CACurrentMediaTime() - started)
        afterTick?()
        refreshAccessibility()
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
            onIdle?()
        }
    }

    // MARK: Accessibility

    /// VoiceOver reads the host's flat snapshot; a change in how many nodes
    /// there are tells it the layout moved.
    private func refreshAccessibility() {
        guard UIAccessibility.isVoiceOverRunning, let host else { return }
        let count = nana_ios_host_accessibility_count(host)
        if count != accessibilityCount {
            accessibilityCount = count
            UIAccessibility.post(notification: .layoutChanged, argument: nil)
        }
    }

    override var isAccessibilityElement: Bool {
        get { false }
        set {}
    }

    override var accessibilityElements: [Any]? {
        get {
            guard let host else { return [] }
            let count = nana_ios_host_accessibility_count(host)
            accessibilityCount = count
            return (0..<count).compactMap { NanaAccessibilityElement(host: host, index: $0, in: self) }
        }
        set {}
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

private let lightImpact = UIImpactFeedbackGenerator(style: .light)
private let mediumImpact = UIImpactFeedbackGenerator(style: .medium)
private let selection = UISelectionFeedbackGenerator()
private let notification = UINotificationFeedbackGenerator()

private let nanaHaptic: @convention(c) (UnsafeMutableRawPointer?, Int32) -> Void = { _, style in
    DispatchQueue.main.async {
        switch Int(style) {
        case NANA_HAPTIC_MEDIUM: mediumImpact.impactOccurred()
        case NANA_HAPTIC_SELECTION: selection.selectionChanged()
        case NANA_HAPTIC_SUCCESS: notification.notificationOccurred(.success)
        case NANA_HAPTIC_WARNING: notification.notificationOccurred(.warning)
        default: lightImpact.impactOccurred()
        }
    }
}

/// One node of the host's accessibility snapshot.
private final class NanaAccessibilityElement: UIAccessibilityElement {
    private let host: OpaquePointer
    private let id: UInt64

    init?(host: OpaquePointer, index: Int, in view: NanaView) {
        var node = NanaAccessibilityNode()
        guard nana_ios_host_accessibility_node(host, index, &node) else { return nil }
        self.host = host
        id = node.id
        super.init(accessibilityContainer: view)
        accessibilityLabel = Self.text(host, index, NANA_AX_TEXT_LABEL)
        accessibilityValue = Self.text(host, index, NANA_AX_TEXT_VALUE)
        accessibilityHint = Self.text(host, index, NANA_AX_TEXT_HINT)
        accessibilityFrameInContainerSpace = CGRect(
            x: CGFloat(node.frame.x), y: CGFloat(node.frame.y),
            width: CGFloat(node.frame.width), height: CGFloat(node.frame.height))
        var traits: UIAccessibilityTraits = []
        switch Int(node.role) {
        case NANA_AX_ROLE_TEXT: traits.insert(.staticText)
        case NANA_AX_ROLE_BUTTON, NANA_AX_ROLE_ROW: traits.insert(.button)
        case NANA_AX_ROLE_TOGGLE:
            traits.insert(.button)
            if #available(iOS 17.0, *) { traits.insert(.toggleButton) }
        case NANA_AX_ROLE_SLIDER: traits.insert(.adjustable)
        case NANA_AX_ROLE_IMAGE: traits.insert(.image)
        case NANA_AX_ROLE_TAB: traits.insert(.button)
        case NANA_AX_ROLE_PROGRESS: traits.insert(.updatesFrequently)
        default: break
        }
        let flags = Int(node.flags)
        if flags & NANA_AX_DISABLED != 0 { traits.insert(.notEnabled) }
        if flags & (NANA_AX_CHECKED | NANA_AX_SELECTED) != 0 { traits.insert(.selected) }
        accessibilityTraits = traits
    }

    private static func text(_ host: OpaquePointer, _ index: Int, _ which: Int) -> String? {
        let length = nana_ios_host_accessibility_text(host, index, Int32(which), nil, 0)
        guard length > 0 else { return nil }
        var bytes = [CChar](repeating: 0, count: Int(length))
        _ = bytes.withUnsafeMutableBufferPointer {
            nana_ios_host_accessibility_text(host, index, Int32(which), $0.baseAddress, Int(length))
        }
        return String(decoding: bytes.map { UInt8(bitPattern: $0) }, as: UTF8.self)
    }

    override func accessibilityActivate() -> Bool {
        nana_ios_host_accessibility_action(host, id, Int32(NANA_AX_ACTION_ACTIVATE))
    }

    override func accessibilityIncrement() {
        _ = nana_ios_host_accessibility_action(host, id, Int32(NANA_AX_ACTION_INCREMENT))
    }

    override func accessibilityDecrement() {
        _ = nana_ios_host_accessibility_action(host, id, Int32(NANA_AX_ACTION_DECREMENT))
    }
}
