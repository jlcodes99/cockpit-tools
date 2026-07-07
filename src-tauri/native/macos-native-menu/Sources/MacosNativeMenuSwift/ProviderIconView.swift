import AppKit
import SwiftUI

enum NativeMenuSwitcherMetrics {
    static let iconSize: CGFloat = 16
    static let titleFontSize: CGFloat = NSFont.smallSystemFontSize - 2
    static let tileCornerRadius: CGFloat = 6
    static let tileHorizontalPadding: CGFloat = 3
    static let tileVerticalPadding: CGFloat = 3
    static let tileSize: CGFloat = 52
    static let tileContentSpacing: CGFloat = 2
    static let gridColumnWidth: CGFloat = 64
    static let gridRowSpacing: CGFloat = 8
    static let sectionTopPadding: CGFloat = 8
    static let sectionBottomPadding: CGFloat = 5
}

enum ProviderIconRenderingMode {
    case template
    case original
}

struct ProviderIconResource {
    let name: String
    let ext: String
    let renderingMode: ProviderIconRenderingMode
}

enum ProviderIconRegistry {
    private static let bundledIconDirectory = "native-menu-icons"

    static func resource(for platformId: String) -> ProviderIconResource? {
        switch platformId {
        case "antigravity":
            return ProviderIconResource(name: "antigravity-menu", ext: "png", renderingMode: .original)
        case "codex":
            return ProviderIconResource(name: "codex", ext: "svg", renderingMode: .template)
        case "claude_manager":
            return ProviderIconResource(name: "claude", ext: "png", renderingMode: .original)
        case "cursor":
            return ProviderIconResource(name: "cursor-menu", ext: "png", renderingMode: .original)
        case "gemini":
            return ProviderIconResource(name: "gemini-menu", ext: "png", renderingMode: .original)
        case "github-copilot":
            return ProviderIconResource(name: "github-copilot", ext: "svg", renderingMode: .template)
        case "kiro":
            return ProviderIconResource(name: "kiro-menu", ext: "png", renderingMode: .original)
        case "windsurf":
            return ProviderIconResource(name: "windsurf", ext: "svg", renderingMode: .original)
        case "zed":
            return ProviderIconResource(name: "zed", ext: "png", renderingMode: .original)
        case "codebuddy", "codebuddy-cn", "codebuddy_cn":
            return ProviderIconResource(name: "codebuddy", ext: "png", renderingMode: .original)
        case "qoder":
            return ProviderIconResource(name: "qoder", ext: "png", renderingMode: .original)
        case "trae":
            return ProviderIconResource(name: "trae", ext: "png", renderingMode: .original)
        case "trae_solo", "trae-solo":
            return ProviderIconResource(name: "trae-solo", ext: "png", renderingMode: .original)
        case "trae_cn", "trae-cn":
            return ProviderIconResource(name: "trae-cn", ext: "png", renderingMode: .original)
        case "trae_solo_cn", "trae-solo-cn":
            return ProviderIconResource(name: "trae-solo-cn", ext: "png", renderingMode: .original)
        case "workbuddy":
            return ProviderIconResource(name: "workbuddy", ext: "png", renderingMode: .original)
        default:
            return nil
        }
    }

    static func image(for platformId: String) -> (image: NSImage, resource: ProviderIconResource)? {
        guard let resource = self.resource(for: platformId),
              let url = Bundle.main.url(
                  forResource: resource.name,
                  withExtension: resource.ext,
                  subdirectory: Self.bundledIconDirectory
              ),
              let data = try? Data(contentsOf: url),
              let image = NSImage(data: data)
        else {
            return nil
        }

        image.size = NSSize(
            width: NativeMenuSwitcherMetrics.iconSize,
            height: NativeMenuSwitcherMetrics.iconSize
        )
        image.isTemplate = resource.renderingMode == .template
        return (image, resource)
    }

    static func monogram(for shortTitle: String) -> String {
        let cleaned = shortTitle
            .replacingOccurrences(of: ".", with: "")
            .replacingOccurrences(of: " ", with: "")
        return String(cleaned.prefix(2)).uppercased()
    }
}

struct ProviderIconView: View {
    let platformId: String
    let shortTitle: String
    let selected: Bool

    var body: some View {
        Group {
            if let resolved = ProviderIconRegistry.image(for: self.platformId) {
                if resolved.resource.renderingMode == .template {
                    Image(nsImage: resolved.image)
                        .renderingMode(.template)
                        .foregroundColor(self.selected ? Color(nsColor: .labelColor) : Color(nsColor: .secondaryLabelColor))
                } else {
                    Image(nsImage: resolved.image)
                        .renderingMode(.original)
                        .interpolation(.high)
                        .antialiased(true)
                }
            } else {
                Text(ProviderIconRegistry.monogram(for: self.shortTitle))
                    .font(.system(size: NativeMenuSwitcherMetrics.titleFontSize))
                    .foregroundColor(self.selected ? Color(nsColor: .labelColor) : Color(nsColor: .secondaryLabelColor))
            }
        }
        .frame(
            width: NativeMenuSwitcherMetrics.iconSize,
            height: NativeMenuSwitcherMetrics.iconSize
        )
    }
}
