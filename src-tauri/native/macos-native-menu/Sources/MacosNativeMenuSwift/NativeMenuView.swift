import AppKit
import SwiftUI

enum NativeMenuLayout {
    static let width: CGFloat = 310
}

enum NativeMenuPalette {
    static let success = Color(hex: "#22C55E")
    static let warning = Color(hex: "#F59E0B")
    static let danger = Color(hex: "#EF4444")
    static let switcherSelectionBackground = Color(nsColor: .controlAccentColor).opacity(0.12)
    static let switcherSelectionStroke = Color(nsColor: .controlAccentColor).opacity(0.42)
    static let switcherHoverBackground = Color(nsColor: .controlColor).opacity(0.55)
}

struct NativeMenuSwitcherSectionView: View {
    @ObservedObject var controller: NativeMenuPopoverController
    let snapshot: NativeMenuSnapshot

    var body: some View {
        ProviderGridSection(
            platforms: self.snapshot.platforms,
            selectedPlatformId: self.controller.selectedPlatformId,
            onSelect: self.controller.selectPlatform(id:)
        )
        .padding(.horizontal, 8)
        .padding(.top, NativeMenuSwitcherMetrics.sectionTopPadding)
        .padding(.bottom, NativeMenuSwitcherMetrics.sectionBottomPadding)
    }
}

struct NativeMenuAccountSectionView: View {
    @ObservedObject var controller: NativeMenuPopoverController
    let platform: NativeMenuPlatform
    let strings: NativeMenuStrings

    private var viewedCard: NativeMenuAccountCard? {
        self.controller.viewedCard
    }

    private var isRefreshingViewedCard: Bool {
        self.controller.isRefreshing(platformId: self.platform.id, accountId: self.viewedCard?.id)
    }

    private var viewedIndexLabel: String {
        guard !self.platform.cards.isEmpty else {
            return "0 / 0"
        }
        let currentId = self.viewedCard?.id
        let index = self.platform.cards.firstIndex(where: { $0.id == currentId }) ?? 0
        return "\(index + 1) / \(self.platform.cards.count)"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 10) {
                PagerButton(systemName: "chevron.left", action: {
                    self.controller.moveViewedAccount(delta: -1)
                })
                .disabled(self.platform.cards.count <= 1 || self.isRefreshingViewedCard)

                Spacer()

                Text(self.viewedIndexLabel)
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundColor(Color(nsColor: .labelColor))
                    .monospacedDigit()

                Spacer()

                PagerButton(systemName: "chevron.right", action: {
                    self.controller.moveViewedAccount(delta: 1)
                })
                .disabled(self.platform.cards.count <= 1 || self.isRefreshingViewedCard)
            }

            if let viewedCard = self.viewedCard {
                AccountHeaderView(platform: self.platform, card: viewedCard)

                VStack(alignment: .leading, spacing: 12) {
                    ForEach(Array(viewedCard.quota_rows.enumerated()), id: \.offset) { index, row in
                        QuotaMetricView(
                            row: row
                        )
                        .equatable()

                        if index < viewedCard.quota_rows.count - 1 {
                            Divider()
                                .overlay(Color(nsColor: .separatorColor).opacity(0.35))
                        }
                    }
                }

                ActionStrip(
                    strings: self.strings,
                    isRefreshing: self.isRefreshingViewedCard,
                    showRecommendedAction: self.controller.shouldShowRecommendedAction(for: self.platform),
                    showBackAction: self.controller.shouldShowBackAction(for: self.platform),
                    showSwitchAction: self.controller.shouldShowSwitchAction(for: self.platform),
                    onViewRecommended: self.controller.jumpToRecommendedAccount,
                    onBackToCurrent: self.controller.jumpBackToCurrentAccount,
                    onRefresh: { self.controller.dispatch(action: .refresh) },
                    onSwitch: { self.controller.dispatch(action: .switchAccount) }
                )
                .padding(.top, 2)
            } else {
                VStack(spacing: 14) {
                    VStack(alignment: .center, spacing: 6) {
                        Text(self.strings.empty_title)
                            .font(.system(size: 17, weight: .semibold))
                            .foregroundColor(Color(nsColor: .labelColor))
                            .multilineTextAlignment(.center)
                            .frame(maxWidth: .infinity)

                        Text(self.strings.empty_desc)
                            .font(.system(size: 13))
                            .foregroundColor(Color(nsColor: .secondaryLabelColor))
                            .multilineTextAlignment(.center)
                            .fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: 220)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 8)

                    HStack {
                        Spacer()
                        ToolbarIconButton(
                            systemName: "arrow.clockwise",
                            spinning: self.controller.isRefreshing(platformId: self.platform.id, accountId: nil),
                            disabled: self.controller.isRefreshing(platformId: self.platform.id, accountId: nil),
                            action: {
                                self.controller.dispatch(action: .refresh)
                            }
                        )
                    }
                }
            }
        }
        .id(self.controller.renderRevision)
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
    }
}

private struct ProviderGridSection: View {
    let platforms: [NativeMenuPlatform]
    let selectedPlatformId: String
    let onSelect: (String) -> Void

    private var columns: [GridItem] {
        Array(
            repeating: GridItem(.fixed(NativeMenuSwitcherMetrics.gridColumnWidth), spacing: 8),
            count: 4
        )
    }

    var body: some View {
        LazyVGrid(columns: self.columns, alignment: .center, spacing: NativeMenuSwitcherMetrics.gridRowSpacing) {
            ForEach(self.platforms) { platform in
                ProviderSwitchTile(
                    platform: platform,
                    selected: platform.id == self.selectedPlatformId,
                    onSelect: { self.onSelect(platform.id) }
                )
            }
        }
    }
}

private struct ProviderSwitchTile: View {
    let platform: NativeMenuPlatform
    let selected: Bool
    let onSelect: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: self.onSelect) {
            VStack(spacing: NativeMenuSwitcherMetrics.tileContentSpacing) {
                ProviderIconView(
                    platformId: self.platform.id,
                    shortTitle: self.platform.short_title,
                    selected: self.selected
                )

                Text(self.platform.short_title)
                    .font(.system(size: NativeMenuSwitcherMetrics.titleFontSize))
                    .foregroundColor(self.titleColor)
                    .multilineTextAlignment(.center)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
                    .allowsTightening(true)
                    .truncationMode(.tail)
                    .frame(width: NativeMenuSwitcherMetrics.tileSize - 6)
            }
            .padding(.horizontal, NativeMenuSwitcherMetrics.tileHorizontalPadding)
            .padding(.vertical, NativeMenuSwitcherMetrics.tileVerticalPadding)
            .frame(
                width: NativeMenuSwitcherMetrics.tileSize,
                height: NativeMenuSwitcherMetrics.tileSize
            )
            .background(
                RoundedRectangle(cornerRadius: NativeMenuSwitcherMetrics.tileCornerRadius, style: .continuous)
                    .fill(self.selectionBackground)
                    .overlay(
                        RoundedRectangle(cornerRadius: NativeMenuSwitcherMetrics.tileCornerRadius, style: .continuous)
                            .strokeBorder(self.selectionStroke, lineWidth: self.selectionStrokeWidth)
                    )
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .frame(maxWidth: .infinity)
        .onHover { inside in
            self.hovering = inside
        }
    }

    private var selectionBackground: Color {
        if self.selected {
            return NativeMenuPalette.switcherSelectionBackground
        }
        if self.hovering {
            return NativeMenuPalette.switcherHoverBackground
        }
        return Color.clear
    }

    private var selectionStroke: Color {
        guard self.selected else {
            return Color.clear
        }
        return NativeMenuPalette.switcherSelectionStroke
    }

    private var selectionStrokeWidth: CGFloat {
        self.selected ? 1 : 0
    }

    private var titleColor: Color {
        if self.selected {
            return Color(nsColor: .labelColor)
        }
        return Color(nsColor: .secondaryLabelColor)
    }
}

private struct AccountHeaderView: View {
    let platform: NativeMenuPlatform
    let card: NativeMenuAccountCard

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(alignment: .firstTextBaseline) {
                Text(self.platform.title)
                    .font(.headline)
                    .fontWeight(.semibold)

                Spacer()

                Text(self.card.title)
                    .font(.subheadline)
                    .foregroundColor(Color(nsColor: .secondaryLabelColor))
                    .lineLimit(1)
                    .truncationMode(.middle)
            }

            HStack(alignment: .firstTextBaseline) {
                Text(self.card.updated_text)
                    .font(.footnote)
                    .foregroundColor(Color(nsColor: .secondaryLabelColor))
                    .lineLimit(1)
                    .minimumScaleFactor(0.75)
                    .allowsTightening(true)

                Spacer()

                if let plan = self.card.plan, !plan.isEmpty {
                    Text(plan)
                        .font(.footnote)
                        .foregroundColor(Color(nsColor: .secondaryLabelColor))
                        .lineLimit(1)
                }
            }
        }
    }
}

private struct QuotaMetricView: View, Equatable {
    let row: NativeMenuQuotaRow

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text(self.row.label)
                    .font(.body)
                    .fontWeight(.medium)
                    .foregroundColor(Color(nsColor: .labelColor))

                Spacer()

                Text(self.row.value)
                    .font(.body)
                    .fontWeight(.medium)
                    .foregroundColor(Color(nsColor: .labelColor))
                    .monospacedDigit()
            }

            if self.row.progress != nil {
                GeometryReader { proxy in
                    ZStack(alignment: .leading) {
                        Capsule()
                            .fill(Color(nsColor: .tertiaryLabelColor).opacity(0.18))

                        Capsule()
                            .fill(self.progressColor)
                            .frame(width: proxy.size.width * CGFloat((self.row.progress ?? 0).clamped(to: 0 ... 100)) / 100.0)
                    }
                }
                .frame(height: 10)
            }

            if let subtext = self.row.subtext, !subtext.isEmpty {
                Text(subtext)
                    .font(.footnote)
                    .foregroundColor(Color(nsColor: .secondaryLabelColor))
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var progressColor: Color {
        switch self.row.progress_tone ?? .high {
        case .high:
            return NativeMenuPalette.success
        case .medium:
            return NativeMenuPalette.warning
        case .low, .critical:
            return NativeMenuPalette.danger
        }
    }
}

private struct ActionStrip: View {
    let strings: NativeMenuStrings
    let isRefreshing: Bool
    let showRecommendedAction: Bool
    let showBackAction: Bool
    let showSwitchAction: Bool
    let onViewRecommended: () -> Void
    let onBackToCurrent: () -> Void
    let onRefresh: () -> Void
    let onSwitch: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            if self.showRecommendedAction {
                ActionCapsuleButton(
                    title: self.strings.view_recommended,
                    icon: "sparkles",
                    emphasized: true,
                    disabled: self.isRefreshing,
                    action: self.onViewRecommended
                )
            }

            if self.showBackAction {
                ActionCapsuleButton(
                    title: self.strings.back_to_current,
                    icon: "arrow.uturn.backward",
                    emphasized: false,
                    disabled: self.isRefreshing,
                    action: self.onBackToCurrent
                )
            }

            if self.showSwitchAction {
                ActionCapsuleButton(
                    title: self.strings.switch_to_viewed,
                    icon: "arrow.left.arrow.right",
                    emphasized: true,
                    disabled: self.isRefreshing,
                    action: self.onSwitch
                )
            }

            Spacer(minLength: 0)

            ToolbarIconButton(
                systemName: "arrow.clockwise",
                spinning: self.isRefreshing,
                disabled: self.isRefreshing,
                action: self.onRefresh
            )
        }
    }
}

private struct ActionCapsuleButton: View {
    let title: String
    let icon: String
    let emphasized: Bool
    let disabled: Bool
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: self.action) {
            HStack(spacing: 6) {
                Image(systemName: self.icon)
                    .font(.system(size: 11, weight: .semibold))

                Text(self.title)
                    .font(.system(size: 12, weight: .semibold))
                    .lineLimit(1)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            .foregroundColor(self.foregroundColor)
            .background(
                Capsule(style: .continuous)
                    .fill(self.backgroundColor)
            )
        }
        .buttonStyle(.plain)
        .disabled(self.disabled)
        .opacity(self.disabled ? 0.56 : 1)
        .onHover { inside in
            self.hovering = inside
        }
    }

    private var foregroundColor: Color {
        self.emphasized ? .white : Color(nsColor: .labelColor)
    }

    private var backgroundColor: Color {
        if self.emphasized {
            return Color(nsColor: .controlAccentColor).opacity(self.hovering && !self.disabled ? 0.88 : 1)
        }
        return Color(nsColor: .controlColor).opacity(self.hovering && !self.disabled ? 0.9 : 0.68)
    }
}

private struct PagerButton: View {
    let systemName: String
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: self.action) {
            Image(systemName: self.systemName)
                .font(.system(size: 12, weight: .semibold))
                .foregroundColor(Color(nsColor: .labelColor))
                .frame(width: 22, height: 22)
                .background(
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(self.hovering ? Color(nsColor: .controlColor) : Color.clear)
                )
        }
        .buttonStyle(.plain)
        .onHover { inside in
            self.hovering = inside
        }
    }
}

private struct ToolbarIconButton: View {
    let systemName: String
    let spinning: Bool
    let disabled: Bool
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: self.action) {
            ZStack {
                if self.spinning {
                    TimelineView(.animation) { context in
                        self.iconImage
                            .rotationEffect(.degrees(self.rotationDegrees(at: context.date)))
                    }
                } else {
                    self.iconImage
                }
            }
            .frame(width: 24, height: 24)
            .background(
                RoundedRectangle(cornerRadius: 7, style: .continuous)
                    .fill(self.hovering && !self.disabled ? Color(nsColor: .controlColor) : Color.clear)
            )
        }
        .buttonStyle(.plain)
        .disabled(self.disabled)
        .opacity(self.disabled && !self.spinning ? 0.78 : 1)
        .onHover { inside in
            self.hovering = inside
        }
    }

    private var iconImage: some View {
        Image(systemName: self.systemName)
            .font(.system(size: 13, weight: .medium))
            .foregroundColor(Color(nsColor: self.spinning ? .labelColor : (self.disabled ? .tertiaryLabelColor : .labelColor)))
    }

    private func rotationDegrees(at date: Date) -> Double {
        date.timeIntervalSinceReferenceDate.remainder(dividingBy: 0.9) / 0.9 * 360
    }
}

private extension Int {
    func clamped(to range: ClosedRange<Int>) -> Int {
        Swift.min(Swift.max(self, range.lowerBound), range.upperBound)
    }
}

private extension Color {
    init(hex: String) {
        let trimmed = hex.trimmingCharacters(in: CharacterSet.alphanumerics.inverted)
        var value: UInt64 = 0
        Scanner(string: trimmed).scanHexInt64(&value)
        let red = Double((value >> 16) & 0xFF) / 255.0
        let green = Double((value >> 8) & 0xFF) / 255.0
        let blue = Double(value & 0xFF) / 255.0
        self.init(.sRGB, red: red, green: green, blue: blue, opacity: 1)
    }
}
