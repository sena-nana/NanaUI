import UIKit

/// The native text proxy (hybrid layer ①): UIKit's text system drives the
/// keyboard, marked text, candidates, the loupe, selection handles and the
/// edit menu (`UITextInteraction`); the focused NanaUI editor holds the text
/// and draws it. The proxy covers the editor while it is focused so UIKit's
/// text gestures land on it, and mirrors the editor through the host.
///
/// Positions are UTF-16 offsets, as UIKit counts; the host takes UTF-8 bytes.
final class NanaTextProxy: UIView, UITextInput {
    weak var nanaView: NanaView?

    private var text = ""
    private var selection = NSRange(location: 0, length: 0)
    private var marked: NSRange?
    private var interaction: UITextInteraction?

    weak var inputDelegate: UITextInputDelegate?
    lazy var tokenizer: UITextInputTokenizer = UITextInputStringTokenizer(textInput: self)
    var markedTextStyle: [NSAttributedString.Key: Any]?
    var autocorrectionType: UITextAutocorrectionType = .no
    var returnKeyType: UIReturnKeyType = .done

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .clear
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override var canBecomeFirstResponder: Bool { true }

    private var host: OpaquePointer? { nanaView?.host }

    // MARK: Editor mirror

    /// The host's focused editor changed: follow it with the keyboard.
    func editorChanged(active: Bool) {
        if active {
            refresh()
            editorMoved()
            if !isFirstResponder {
                becomeFirstResponder()
                let interaction = UITextInteraction(for: .editable)
                interaction.textInput = self
                addInteraction(interaction)
                self.interaction = interaction
            }
        } else if isFirstResponder || interaction != nil {
            if let interaction { removeInteraction(interaction) }
            interaction = nil
            resignFirstResponder()
            frame = .zero
        }
    }

    /// Cover the focused editor, in the NanaUI view's points.
    func editorMoved() {
        guard let host, interaction != nil else { return }
        var rect = NanaRect()
        if nana_ios_host_text_bounds(host, &rect) {
            frame = CGRect(x: CGFloat(rect.x), y: CGFloat(rect.y), width: CGFloat(rect.width), height: CGFloat(rect.height))
        }
    }

    private func refresh() {
        guard let host else { return }
        var selStart: Int64 = -1, selEnd: Int64 = -1, markStart: Int64 = -1, markEnd: Int64 = -1
        let length = nana_ios_host_text_snapshot(host, nil, 0, &selStart, &selEnd, &markStart, &markEnd)
        guard length >= 0 else { return }
        var bytes = [UInt8](repeating: 0, count: Int(length))
        _ = bytes.withUnsafeMutableBufferPointer { buffer in
            nana_ios_host_text_snapshot(host, buffer.baseAddress, buffer.count, &selStart, &selEnd, &markStart, &markEnd)
        }
        text = String(decoding: bytes, as: UTF8.self)
        let start = utf16(Int(selStart)), end = utf16(Int(selEnd))
        selection = NSRange(location: min(start, end), length: abs(end - start))
        marked = markStart >= 0 ? NSRange(location: utf16(Int(markStart)), length: utf16(Int(markEnd)) - utf16(Int(markStart))) : nil
    }

    /// Change the editor through the host, then tell UIKit.
    private func edit(_ change: (OpaquePointer) -> Void) {
        guard let host else { return }
        inputDelegate?.textWillChange(self)
        inputDelegate?.selectionWillChange(self)
        change(host)
        refresh()
        inputDelegate?.selectionDidChange(self)
        inputDelegate?.textDidChange(self)
        nanaView?.requestFrame()
    }

    private func commit(_ string: String) {
        edit { host in
            var string = string
            string.withUTF8 { nana_ios_host_commit_text(host, $0.baseAddress, $0.count) }
        }
    }

    // MARK: Offsets

    private func utf8(_ utf16Offset: Int) -> Int {
        let index = String.Index(utf16Offset: max(0, min(utf16Offset, text.utf16.count)), in: text)
        return text.utf8.distance(from: text.startIndex, to: index)
    }

    private func utf16(_ utf8Offset: Int) -> Int {
        let index = text.utf8.index(text.startIndex, offsetBy: max(0, min(utf8Offset, text.utf8.count)))
        return text.utf16.distance(from: text.startIndex, to: index)
    }

    private func offset(_ position: UITextPosition) -> Int {
        (position as? NanaTextPosition)?.offset ?? 0
    }

    private func range(_ range: UITextRange) -> NSRange {
        let start = offset(range.start), end = offset(range.end)
        return NSRange(location: min(start, end), length: abs(end - start))
    }

    private func clamp(_ offset: Int) -> Int { max(0, min(offset, text.utf16.count)) }

    // MARK: UIKeyInput

    var hasText: Bool { !text.isEmpty }

    func insertText(_ string: String) {
        if string == "\n" {
            nanaView?.endEditing(true)
            return
        }
        commit(string)
    }

    func deleteBackward() {
        if let marked {
            let utf16 = Array(text.utf16)[marked.location..<marked.location + marked.length]
            let shorter = String(decoding: utf16.dropLast(), as: UTF16.self)
            setMarkedText(shorter, selectedRange: NSRange(location: shorter.utf16.count, length: 0))
            return
        }
        if selection.length > 0 {
            commit("")
            return
        }
        let caret = String.Index(utf16Offset: selection.location, in: text)
        guard caret > text.startIndex else { return }
        let previous = text.index(before: caret)
        let bytes = text.utf8.distance(from: previous, to: caret)
        edit { nana_ios_host_delete_surrounding($0, bytes, 0) }
    }

    // MARK: Text and selection

    func text(in range: UITextRange) -> String? {
        let range = self.range(range)
        let utf16 = Array(text.utf16)
        guard range.location + range.length <= utf16.count else { return nil }
        return String(decoding: utf16[range.location..<range.location + range.length], as: UTF16.self)
    }

    func replace(_ range: UITextRange, withText text: String) {
        let range = self.range(range)
        edit { nana_ios_host_set_selection($0, utf8(range.location), utf8(range.location + range.length)) }
        commit(text)
    }

    var selectedTextRange: UITextRange? {
        get { NanaTextRange(selection) }
        set {
            guard let newValue, marked == nil else { return }
            let range = self.range(newValue)
            edit { nana_ios_host_set_selection($0, utf8(range.location), utf8(range.location + range.length)) }
        }
    }

    var markedTextRange: UITextRange? { marked.map(NanaTextRange.init) }

    func setMarkedText(_ markedText: String?, selectedRange: NSRange) {
        var marked = markedText ?? ""
        let start = marked.utf8.distance(
            from: marked.startIndex, to: String.Index(utf16Offset: selectedRange.location, in: marked))
        let end = marked.utf8.distance(
            from: marked.startIndex,
            to: String.Index(utf16Offset: selectedRange.location + selectedRange.length, in: marked))
        edit { host in
            marked.withUTF8 { nana_ios_host_set_marked_text(host, $0.baseAddress, $0.count, Int64(start), Int64(end)) }
        }
    }

    func unmarkText() {
        guard let marked, let range = textRange(from: NanaTextPosition(marked.location), to: NanaTextPosition(marked.location + marked.length)),
              let committed = text(in: range)
        else { return }
        commit(committed)
    }

    // MARK: Positions

    var beginningOfDocument: UITextPosition { NanaTextPosition(0) }
    var endOfDocument: UITextPosition { NanaTextPosition(text.utf16.count) }

    func textRange(from fromPosition: UITextPosition, to toPosition: UITextPosition) -> UITextRange? {
        let start = offset(fromPosition), end = offset(toPosition)
        return NanaTextRange(NSRange(location: min(start, end), length: abs(end - start)))
    }

    func position(from position: UITextPosition, offset: Int) -> UITextPosition? {
        let target = self.offset(position) + offset
        guard (0...text.utf16.count).contains(target) else { return nil }
        return NanaTextPosition(target)
    }

    func position(from position: UITextPosition, in direction: UITextLayoutDirection, offset: Int) -> UITextPosition? {
        switch direction {
        case .left, .up: return self.position(from: position, offset: -offset)
        default: return self.position(from: position, offset: offset)
        }
    }

    func compare(_ position: UITextPosition, to other: UITextPosition) -> ComparisonResult {
        let left = offset(position), right = offset(other)
        return left < right ? .orderedAscending : left > right ? .orderedDescending : .orderedSame
    }

    func offset(from: UITextPosition, to toPosition: UITextPosition) -> Int {
        offset(toPosition) - offset(from)
    }

    func position(within range: UITextRange, farthestIn direction: UITextLayoutDirection) -> UITextPosition? {
        switch direction {
        case .left, .up: return range.start
        default: return range.end
        }
    }

    func characterRange(byExtending position: UITextPosition, in direction: UITextLayoutDirection) -> UITextRange? {
        switch direction {
        case .left, .up: return textRange(from: beginningOfDocument, to: position)
        default: return textRange(from: position, to: endOfDocument)
        }
    }

    func baseWritingDirection(for position: UITextPosition, in direction: UITextStorageDirection) -> NSWritingDirection {
        .natural
    }

    func setBaseWritingDirection(_ writingDirection: NSWritingDirection, for range: UITextRange) {}

    // MARK: Geometry (the host's text layout, in the proxy's coordinates)

    func caretRect(for position: UITextPosition) -> CGRect {
        guard let host, let nanaView else { return .zero }
        var rect = NanaRect()
        guard nana_ios_host_text_caret_rect(host, utf8(offset(position)), &rect) else { return .zero }
        let caret = CGRect(x: CGFloat(rect.x), y: CGFloat(rect.y), width: 2, height: CGFloat(rect.height))
        return convert(caret, from: nanaView)
    }

    func firstRect(for range: UITextRange) -> CGRect {
        let start = caretRect(for: range.start), end = caretRect(for: range.end)
        guard abs(start.midY - end.midY) < start.height / 2, end.maxX > start.minX else { return start }
        return CGRect(x: start.minX, y: start.minY, width: end.minX - start.minX, height: start.height)
    }

    func selectionRects(for range: UITextRange) -> [UITextSelectionRect] {
        [NanaSelectionRect(rect: firstRect(for: range), containsStart: true, containsEnd: true)]
    }

    func closestPosition(to point: CGPoint) -> UITextPosition? {
        guard let host, let nanaView else { return nil }
        let local = convert(point, to: nanaView)
        let hit = nana_ios_host_text_offset_at(host, Float(local.x), Float(local.y))
        return hit >= 0 ? NanaTextPosition(utf16(Int(hit))) : nil
    }

    func closestPosition(to point: CGPoint, within range: UITextRange) -> UITextPosition? {
        let range = self.range(range)
        let hit = (closestPosition(to: point) as? NanaTextPosition)?.offset ?? range.location
        return NanaTextPosition(max(range.location, min(hit, range.location + range.length)))
    }

    func characterRange(at point: CGPoint) -> UITextRange? {
        guard let position = closestPosition(to: point) as? NanaTextPosition else { return nil }
        let start = clamp(position.offset)
        return NanaTextRange(NSRange(location: start, length: start < text.utf16.count ? 1 : 0))
    }
}

final class NanaTextPosition: UITextPosition {
    let offset: Int

    init(_ offset: Int) { self.offset = offset }
}

final class NanaTextRange: UITextRange {
    let range: NSRange

    init(_ range: NSRange) { self.range = range }

    override var start: UITextPosition { NanaTextPosition(range.location) }
    override var end: UITextPosition { NanaTextPosition(range.location + range.length) }
    override var isEmpty: Bool { range.length == 0 }
}

final class NanaSelectionRect: UITextSelectionRect {
    private let storedRect: CGRect
    private let storedStart: Bool
    private let storedEnd: Bool

    init(rect: CGRect, containsStart: Bool, containsEnd: Bool) {
        storedRect = rect
        storedStart = containsStart
        storedEnd = containsEnd
    }

    override var rect: CGRect { storedRect }
    override var writingDirection: NSWritingDirection { .natural }
    override var containsStart: Bool { storedStart }
    override var containsEnd: Bool { storedEnd }
    override var isVertical: Bool { false }
}
