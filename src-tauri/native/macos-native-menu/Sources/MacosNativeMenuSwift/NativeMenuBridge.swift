import AppKit
import CoreFoundation
import Foundation

@_silgen_name("macos_native_menu_dispatch_action")
private func rustDispatchMenuAction(
    _ action: UnsafePointer<CChar>?,
    _ platformId: UnsafePointer<CChar>?,
    _ accountId: UnsafePointer<CChar>?
)

private func runNativeMenuControllerSync(
    label: String,
    _ operation: @escaping @MainActor () -> Void
) {
    precondition(Thread.isMainThread, "[NativeMenu] \(label) 必须在主线程执行")
    MainActor.assumeIsolated {
        operation()
    }
}

private let nativeMenuRunLoopModes: [RunLoop.Mode] = [
    .default,
    .eventTracking,
    .modalPanel,
]

private func runNativeMenuController(
    label: String,
    _ operation: @escaping @MainActor () -> Void
) {
    if Thread.isMainThread {
        runNativeMenuControllerSync(label: label, operation)
        return
    }

    RunLoop.main.perform(inModes: nativeMenuRunLoopModes) {
        runNativeMenuControllerSync(label: label, operation)
    }
    CFRunLoopWakeUp(CFRunLoopGetMain())
}

func dispatchRustMenuAction(action: String, platformId: String? = nil, accountId: String? = nil) {
    action.withCString { actionPointer in
        if let platformId {
            platformId.withCString { platformPointer in
                if let accountId {
                    accountId.withCString { accountPointer in
                        rustDispatchMenuAction(actionPointer, platformPointer, accountPointer)
                    }
                } else {
                    rustDispatchMenuAction(actionPointer, platformPointer, nil)
                }
            }
        } else {
            rustDispatchMenuAction(actionPointer, nil, nil)
        }
    }
}

@_cdecl("macos_native_menu_toggle")
public func macos_native_menu_toggle(
    snapshotJSONPointer: UnsafePointer<CChar>?,
    statusItemPointer: UnsafeMutableRawPointer?
) {
    guard let snapshotJSONPointer, let statusItemPointer else { return }
    let snapshotJSON = String(cString: snapshotJSONPointer)
    runNativeMenuController(label: "toggle") {
        NativeMenuPopoverController.shared.toggle(
            snapshotJSON: snapshotJSON,
            statusItemPointer: statusItemPointer
        )
    }
}

@_cdecl("macos_native_menu_update_snapshot")
public func macos_native_menu_update_snapshot(
    snapshotJSONPointer: UnsafePointer<CChar>?
) {
    guard let snapshotJSONPointer else { return }
    let snapshotJSON = String(cString: snapshotJSONPointer)
    runNativeMenuController(label: "update_snapshot") {
        NativeMenuPopoverController.shared.update(snapshotJSON: snapshotJSON)
    }
}

/// tray-icon 在 NSStatusBarButton 上叠加 `TaoTrayTarget` 子视图接收鼠标事件。
/// 直接改 button 的 title/attributedTitle 会拉宽状态栏项，但不会同步放大该点击层，
/// 结果是图标/文字区域“看起来在”，左右键却完全无响应。
func syncTrayClickTargetFrame(for button: NSStatusBarButton) {
    button.layoutSubtreeIfNeeded()
    let bounds = button.bounds
    guard bounds.width > 0, bounds.height > 0 else { return }

    var matched = false
    for subview in button.subviews {
        let className = NSStringFromClass(type(of: subview))
        // tray-icon 注册名为 TaoTrayTarget；兼容可能的命名变化。
        if className.contains("TaoTrayTarget") || className.contains("TrayTarget") {
            subview.frame = bounds
            subview.autoresizingMask = [.width, .height]
            matched = true
        }
    }

    // 兜底：若类名匹配失败，只同步覆盖整个 button 的透明事件层（通常仅此一个子视图）。
    if !matched, button.subviews.count == 1, let only = button.subviews.first {
        only.frame = bounds
        only.autoresizingMask = [.width, .height]
    }
}

func syncTrayClickTargetFrameSoon(for button: NSStatusBarButton) {
    syncTrayClickTargetFrame(for: button)
    // 状态栏变宽后 bounds 有时会在下一帧才稳定，补一次异步对齐。
    DispatchQueue.main.async {
        syncTrayClickTargetFrame(for: button)
    }
}

private func adaptiveStatusBarIcon(_ image: NSImage, appearance: NSAppearance) -> NSImage {
    let size = NSSize(width: 16, height: 16)
    let tinted = NSImage(size: size)
    appearance.performAsCurrentDrawingAppearance {
        tinted.lockFocus()
        image.draw(in: NSRect(origin: .zero, size: size))
        NSColor.labelColor.setFill()
        NSRect(origin: .zero, size: size).fill(using: .sourceIn)
        tinted.unlockFocus()
    }
    return tinted
}

@_cdecl("macos_native_menu_update_status_item")
public func macos_native_menu_update_status_item(
    statusesJSONPointer: UnsafePointer<CChar>?,
    enabled: Int32,
    monochromeEnabled: Int32,
    statusItemPointer: UnsafeMutableRawPointer?
) {
    guard let statusItemPointer else { return }
    let statuses: [NativeMenuBarStatus] = statusesJSONPointer
        .map(String.init(cString:))
        .flatMap { $0.data(using: .utf8) }
        .flatMap { try? JSONDecoder().decode([NativeMenuBarStatus].self, from: $0) }
        ?? []

    runNativeMenuController(label: "update_status_item") {
        let statusItem = Unmanaged<NSStatusItem>
            .fromOpaque(statusItemPointer)
            .takeUnretainedValue()
        guard let button = statusItem.button else { return }

        statusItem.length = NSStatusItem.variableLength
        guard enabled != 0, !statuses.isEmpty else {
            button.wantsLayer = true
            button.layer?.backgroundColor = NSColor.clear.cgColor
            button.layer?.cornerRadius = 0
            button.attributedTitle = NSAttributedString(string: "")
            button.title = ""
            button.imagePosition = .imageOnly
            button.needsDisplay = true
            syncTrayClickTargetFrameSoon(for: button)
            return
        }

        let font = NSFont.monospacedDigitSystemFont(
            ofSize: NSFont.systemFontSize,
            weight: .medium
        )
        let attributedTitle = NSMutableAttributedString(string: "")

        for (index, status) in statuses.enumerated() {
            if index > 0 {
                attributedTitle.append(NSAttributedString(string: "  "))
            }

            let resolvedIcon = ProviderIconRegistry.image(for: status.platform_id)
            let icon = resolvedIcon.map {
                $0.resource.renderingMode == .template
                    ? adaptiveStatusBarIcon($0.image, appearance: button.effectiveAppearance)
                    : $0.image
            } ?? NSImage(
                systemSymbolName: "questionmark.square.dashed",
                accessibilityDescription: status.short_title
            ).map { adaptiveStatusBarIcon($0, appearance: button.effectiveAppearance) }
            if let icon {
                let attachment = NSTextAttachment()
                attachment.image = icon
                attachment.bounds = NSRect(x: 0, y: -3, width: 16, height: 16)
                attributedTitle.append(NSAttributedString(attachment: attachment))
            }

            let valueText = status.value_text.isEmpty ? "--" : status.value_text
            let valueColor: NSColor
            if monochromeEnabled != 0 {
                valueColor = .labelColor
            } else if let remainingPercent = status.remaining_percent {
                let tone = min(max(remainingPercent, 0), 100)
                if tone <= 30 {
                    valueColor = .systemRed
                } else if tone <= 60 {
                    valueColor = .systemOrange
                } else {
                    valueColor = .systemGreen
                }
            } else {
                valueColor = .secondaryLabelColor
            }
            attributedTitle.append(NSAttributedString(
                string: " \(valueText)",
                attributes: [
                    .font: font,
                    .foregroundColor: valueColor,
                ]
            ))
        }

        button.imagePosition = .noImage
        button.title = ""
        button.attributedTitle = attributedTitle
        button.wantsLayer = true
        button.layer?.backgroundColor = NSColor.clear.cgColor
        button.layer?.cornerRadius = 0
        button.invalidateIntrinsicContentSize()
        button.layoutSubtreeIfNeeded()
        statusItem.length = ceil(button.fittingSize.width) + 8
        button.needsDisplay = true
        // 标题变宽后必须立刻同步点击层，否则左右键全部失效。
        syncTrayClickTargetFrameSoon(for: button)
    }
}
