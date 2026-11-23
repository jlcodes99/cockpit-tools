import AppKit

/// A status button positions its title on the system's single-line baseline.
/// Draw the compact title in a noninteractive layer instead, centering the whole
/// text block in the button's current bounds while keeping the native icon/cell.
final class NativeMenuQuotaTitleView: NSView {
    private var accountPrefix = ""
    private var valueText = ""
    private var remainingPercent: Int32 = -1
    private var title = NSAttributedString(string: "")
    private var lastHeight: CGFloat = -1
    private weak var button: NSButton?

    override var isFlipped: Bool { true }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    init(button: NSButton) {
        self.button = button
        super.init(frame: button.bounds)
        autoresizingMask = [.width, .height]
        setAccessibilityElement(false)
    }

    required init?(coder: NSCoder) { nil }

    func configure(accountPrefix: String, valueText: String, remainingPercent: Int32) {
        self.accountPrefix = accountPrefix
        self.valueText = valueText
        self.remainingPercent = remainingPercent
        lastHeight = -1
        updateTitle()
        needsDisplay = true
    }

    override func layout() {
        super.layout()
        updateTitle()
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        updateTitle()
        needsDisplay = true
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        needsDisplay = true
    }

    private func updateTitle() {
        guard let button, lastHeight != bounds.height else { return }
        lastHeight = bounds.height
        title = menuBarQuotaTitle(
            accountPrefix: accountPrefix,
            valueText: valueText,
            remainingPercent: remainingPercent,
            stacked: true,
            availableHeight: bounds.height
        )
        // Keep AppKit's intrinsic width calculation, hiding only its title paint.
        let transparentTitle = NSMutableAttributedString(attributedString: title)
        transparentTitle.addAttribute(.foregroundColor, value: NSColor.clear,
            range: NSRange(location: 0, length: transparentTitle.length))
        button.attributedTitle = transparentTitle
        if let statusButton = button as? NSStatusBarButton {
            DispatchQueue.main.async { [weak statusButton] in
                if let statusButton { syncTrayClickTargetFrame(for: statusButton) }
            }
        }
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let button, let cell = button.cell else { return }
        let titleRect = convert(cell.titleRect(forBounds: button.bounds), from: button)
        let rect = Self.drawingRect(
            bounds: bounds,
            titleRect: titleRect,
            textSize: title.size(),
            scale: window?.backingScaleFactor ?? 1
        )
        title.draw(with: rect, options: [.usesLineFragmentOrigin, .usesFontLeading])
    }

    static func drawingRect(bounds: NSRect, titleRect: NSRect, textSize: NSSize, scale: CGFloat) -> NSRect {
        let pixelScale = max(1, scale)
        let y = ((bounds.midY - textSize.height / 2) * pixelScale).rounded() / pixelScale
        return NSRect(x: titleRect.minX, y: y,
            width: max(titleRect.width, textSize.width), height: textSize.height)
    }
}
