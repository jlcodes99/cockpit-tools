import AppKit

/// Keep the tray button, icon and click target native; only change its title's typography.
func menuBarQuotaTitle(
    accountPrefix: String,
    valueText: String,
    remainingPercent: Int32,
    stacked: Bool,
    availableHeight: CGFloat = 22
) -> NSAttributedString {
    let value = valueText.isEmpty ? "--" : valueText
    let windows = value.components(separatedBy: " · ")
    let prefix = accountPrefix.isEmpty ? "" : "\(accountPrefix) "
    let twoLines = stacked && windows.count == 2
    // Keep 10pt text readable in the smallest standard menu bar; use a little
    // more of the available space on taller menu bars, without growing unbounded.
    let compactFontSize = max(10, min(11, floor((availableHeight - 2) / 2) - 1))
    let font = NSFont.monospacedDigitSystemFont(
        ofSize: twoLines ? compactFontSize : NSFont.systemFontSize,
        weight: .medium
    )
    let measure: (String) -> CGFloat = { ($0 as NSString).size(withAttributes: [.font: font]).width }
    let columns = windows.map { window -> (label: String, amount: String) in
        guard let separator = window.lastIndex(of: " ") else { return (window, "") }
        return (String(window[..<separator]), String(window[window.index(after: separator)...]))
    }
    let text: String
    if twoLines {
        text = prefix + columns.map { "\($0.label)\t\($0.amount)" }.joined(separator: "\n")
    } else {
        text = prefix + value
    }
    // One quota always falls back to the normal single line, including account/API labels.
    let paragraph = NSMutableParagraphStyle()
    paragraph.alignment = twoLines ? .left : .center
    paragraph.lineBreakMode = .byClipping
    if twoLines {
        paragraph.minimumLineHeight = compactFontSize + 1
        paragraph.maximumLineHeight = compactFontSize + 1
        let labelWidth = columns.map { measure($0.label) }.max() ?? 0
        let amountWidth = columns.map { measure($0.amount) }.max() ?? 0
        let rightEdge = ceil(measure(prefix) + labelWidth + 4 + amountWidth)
        paragraph.tabStops = [NSTextTab(textAlignment: .right, location: rightEdge, options: [:])]
    }
    let title = NSMutableAttributedString(string: text, attributes: [
        .font: font,
        .foregroundColor: NSColor.labelColor,
        .paragraphStyle: paragraph,
    ])
    if twoLines && !prefix.isEmpty {
        // Keep both window labels aligned after the account label on the first row.
        let secondLineStyle = paragraph.mutableCopy() as! NSMutableParagraphStyle
        secondLineStyle.firstLineHeadIndent = measure(prefix)
        let start = (text as NSString).range(of: "\n").location + 1
        title.addAttribute(.paragraphStyle, value: secondLineStyle, range: NSRange(
            location: start, length: title.length - start
        ))
    }

    let valueColor: NSColor
    if remainingPercent < 0 {
        valueColor = .secondaryLabelColor
    } else {
        switch min(max(Int(remainingPercent), 0), 100) {
        case ...30: valueColor = .systemRed
        case ...60: valueColor = .systemOrange
        default: valueColor = .systemGreen
        }
    }
    let prefixLength = accountPrefix.isEmpty ? 0 : accountPrefix.utf16.count + 1
    title.addAttribute(.foregroundColor, value: valueColor, range: NSRange(
        location: prefixLength,
        length: title.length - prefixLength
    ))
    return title
}
