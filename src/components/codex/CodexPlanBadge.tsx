import type { CSSProperties } from "react";
import {
  Aperture,
  Badge,
  BadgeCheck,
  Binary,
  BrainCircuit,
  CircuitBoard,
  Cpu,
  Bolt,
  CircleDollarSign,
  Crown,
  Diamond,
  Flame,
  Fingerprint,
  Gem,
  Hexagon,
  Medal,
  Network,
  Orbit,
  QrCode,
  Radar,
  RadioTower,
  Rocket,
  Satellite,
  ScanLine,
  ShieldCheck,
  Sparkles,
  Star,
  Trophy,
  WandSparkles,
  Webhook,
  Zap,
  type LucideIcon,
} from "lucide-react";

export const CODEX_PLAN_BADGE_STYLE_STORAGE_KEY =
  "agtools.codex.plan_badge_styles.v1";

export const CODEX_PLAN_BADGE_TIERS = [
  "free",
  "team",
  "proLite",
  "pro",
  "plus",
] as const;

export type CodexPlanBadgeTier = (typeof CODEX_PLAN_BADGE_TIERS)[number];
export type CodexPlanBadgeStyleId = `style-${number}`;

type CodexPlanBadgeIconKey =
  | "aperture"
  | "badge"
  | "badge-check"
  | "binary"
  | "bolt"
  | "brain-circuit"
  | "circle-dollar"
  | "circuit-board"
  | "cpu"
  | "crown"
  | "diamond"
  | "fingerprint"
  | "flame"
  | "gem"
  | "hexagon"
  | "medal"
  | "network"
  | "orbit"
  | "qr-code"
  | "radar"
  | "radio-tower"
  | "rocket"
  | "satellite"
  | "scan-line"
  | "shield-check"
  | "sparkles"
  | "star"
  | "trophy"
  | "wand-sparkles"
  | "webhook"
  | "zap";

interface CodexPlanBadgeTheme {
  bg: string;
  color: string;
  border: string;
  shadow: string;
  iconBg: string;
  iconColor: string;
  iconShadow: string;
  labelShadow: string;
}

export interface CodexPlanBadgeStyle {
  id: CodexPlanBadgeStyleId;
  name: string;
  icon: CodexPlanBadgeIconKey | null;
  theme: CodexPlanBadgeTheme;
}

export type CodexPlanBadgeStylePreferences = Record<
  CodexPlanBadgeTier,
  CodexPlanBadgeStyleId
>;

export const DEFAULT_CODEX_PLAN_BADGE_STYLE_PREFERENCES: CodexPlanBadgeStylePreferences =
  {
    free: "style-1",
    team: "style-1",
    proLite: "style-1",
    pro: "style-1",
    plus: "style-1",
  };

export const CODEX_PLAN_BADGE_TIER_META: Record<
  CodexPlanBadgeTier,
  { label: string; previewLabel: string; planClass: string }
> = {
  free: { label: "FREE", previewLabel: "FREE", planClass: "free" },
  team: { label: "TEAM", previewLabel: "TEAM", planClass: "team" },
  proLite: {
    label: "PRO 5X",
    previewLabel: "PRO 5x",
    planClass: "pro codex-pro-lite",
  },
  pro: {
    label: "PRO 20X",
    previewLabel: "PRO 20x",
    planClass: "pro codex-pro-max",
  },
  plus: { label: "PLUS", previewLabel: "PLUS", planClass: "plus codex-plus" },
};

const CODEX_PLAN_BADGE_ICONS: Record<CodexPlanBadgeIconKey, LucideIcon> = {
  aperture: Aperture,
  badge: Badge,
  "badge-check": BadgeCheck,
  binary: Binary,
  bolt: Bolt,
  "brain-circuit": BrainCircuit,
  "circle-dollar": CircleDollarSign,
  "circuit-board": CircuitBoard,
  cpu: Cpu,
  crown: Crown,
  diamond: Diamond,
  fingerprint: Fingerprint,
  flame: Flame,
  gem: Gem,
  hexagon: Hexagon,
  medal: Medal,
  network: Network,
  orbit: Orbit,
  "qr-code": QrCode,
  radar: Radar,
  "radio-tower": RadioTower,
  rocket: Rocket,
  satellite: Satellite,
  "scan-line": ScanLine,
  "shield-check": ShieldCheck,
  sparkles: Sparkles,
  star: Star,
  trophy: Trophy,
  "wand-sparkles": WandSparkles,
  webhook: Webhook,
  zap: Zap,
};

function defineBadgeStyle(
  id: CodexPlanBadgeStyleId,
  name: string,
  icon: CodexPlanBadgeIconKey | null,
  theme: CodexPlanBadgeTheme,
): CodexPlanBadgeStyle {
  return { id, name, icon, theme };
}

const CODEX_PLAN_TECH_VARIANTS = [
  {
    name: "量子矩阵",
    icon: "cpu",
    dark: "#020617",
    mid: "#12335f",
    accent: "#22d3ee",
    soft: "#cffafe",
    glow: "rgba(34, 211, 238, .34)",
  },
  {
    name: "霓虹回路",
    icon: "circuit-board",
    dark: "#06141f",
    mid: "#0f766e",
    accent: "#5eead4",
    soft: "#ccfbf1",
    glow: "rgba(94, 234, 212, .32)",
  },
  {
    name: "深空雷达",
    icon: "radar",
    dark: "#0f172a",
    mid: "#312e81",
    accent: "#818cf8",
    soft: "#e0e7ff",
    glow: "rgba(129, 140, 248, .34)",
  },
  {
    name: "星轨卫星",
    icon: "satellite",
    dark: "#111827",
    mid: "#1d4ed8",
    accent: "#93c5fd",
    soft: "#dbeafe",
    glow: "rgba(147, 197, 253, .32)",
  },
  {
    name: "代码脉冲",
    icon: "binary",
    dark: "#052e16",
    mid: "#16a34a",
    accent: "#bef264",
    soft: "#ecfccb",
    glow: "rgba(190, 242, 100, .32)",
  },
  {
    name: "扫描光栅",
    icon: "scan-line",
    dark: "#172554",
    mid: "#2563eb",
    accent: "#67e8f9",
    soft: "#e0f2fe",
    glow: "rgba(103, 232, 249, .34)",
  },
  {
    name: "轨道节点",
    icon: "orbit",
    dark: "#1e1b4b",
    mid: "#7c3aed",
    accent: "#f0abfc",
    soft: "#fae8ff",
    glow: "rgba(240, 171, 252, .32)",
  },
  {
    name: "神经核心",
    icon: "brain-circuit",
    dark: "#0f172a",
    mid: "#be123c",
    accent: "#fb7185",
    soft: "#ffe4e6",
    glow: "rgba(251, 113, 133, .3)",
  },
  {
    name: "指纹密钥",
    icon: "fingerprint",
    dark: "#18181b",
    mid: "#57534e",
    accent: "#facc15",
    soft: "#fef9c3",
    glow: "rgba(250, 204, 21, .3)",
  },
  {
    name: "网络中枢",
    icon: "network",
    dark: "#042f2e",
    mid: "#0891b2",
    accent: "#a7f3d0",
    soft: "#ecfdf5",
    glow: "rgba(167, 243, 208, .3)",
  },
  {
    name: "接口跃迁",
    icon: "webhook",
    dark: "#12071f",
    mid: "#6d28d9",
    accent: "#60a5fa",
    soft: "#dbeafe",
    glow: "rgba(96, 165, 250, .32)",
  },
  {
    name: "信标塔台",
    icon: "radio-tower",
    dark: "#1c1917",
    mid: "#b45309",
    accent: "#fdba74",
    soft: "#ffedd5",
    glow: "rgba(253, 186, 116, .3)",
  },
  {
    name: "虹膜棱镜",
    icon: "aperture",
    dark: "#020617",
    mid: "#0e7490",
    accent: "#c084fc",
    soft: "#f3e8ff",
    glow: "rgba(192, 132, 252, .3)",
  },
  {
    name: "二维码芯片",
    icon: "qr-code",
    dark: "#030712",
    mid: "#334155",
    accent: "#38bdf8",
    soft: "#e0f2fe",
    glow: "rgba(56, 189, 248, .32)",
  },
  {
    name: "零点推进",
    icon: "zap",
    dark: "#052e2b",
    mid: "#0d9488",
    accent: "#fde047",
    soft: "#fef9c3",
    glow: "rgba(253, 224, 71, .3)",
  },
] satisfies Array<{
  name: string;
  icon: CodexPlanBadgeIconKey;
  dark: string;
  mid: string;
  accent: string;
  soft: string;
  glow: string;
}>;

const CODEX_PLAN_TECH_TIER_SUFFIX: Record<CodexPlanBadgeTier, string> = {
  free: "基础",
  team: "协同",
  proLite: "轻量",
  pro: "旗舰",
  plus: "增幅",
};

const CODEX_PLAN_GOLD_VARIANTS = [
  {
    name: "鎏金王冠",
    icon: "crown",
    dark: "#241507",
    mid: "#9a5d08",
    accent: "#facc15",
    soft: "#fff7c2",
    glow: "rgba(250, 204, 21, .38)",
  },
  {
    name: "曜金星徽",
    icon: "star",
    dark: "#1c1917",
    mid: "#b7791f",
    accent: "#fde047",
    soft: "#fef9c3",
    glow: "rgba(253, 224, 71, .36)",
  },
  {
    name: "金砂奖杯",
    icon: "trophy",
    dark: "#29220c",
    mid: "#ca8a04",
    accent: "#fbbf24",
    soft: "#fffbeb",
    glow: "rgba(251, 191, 36, .36)",
  },
  {
    name: "熔金钻面",
    icon: "diamond",
    dark: "#1f1206",
    mid: "#c27803",
    accent: "#ffd166",
    soft: "#fff3bf",
    glow: "rgba(255, 209, 102, .34)",
  },
  {
    name: "金箔盾章",
    icon: "shield-check",
    dark: "#211806",
    mid: "#b45309",
    accent: "#fcd34d",
    soft: "#fef3c7",
    glow: "rgba(252, 211, 77, .35)",
  },
  {
    name: "圣辉星芒",
    icon: "sparkles",
    dark: "#2a1705",
    mid: "#d97706",
    accent: "#fde68a",
    soft: "#fff7ed",
    glow: "rgba(253, 230, 138, .36)",
  },
  {
    name: "暗金晶核",
    icon: "gem",
    dark: "#0f0b05",
    mid: "#854d0e",
    accent: "#eab308",
    soft: "#fef08a",
    glow: "rgba(234, 179, 8, .34)",
  },
  {
    name: "琥珀闪电",
    icon: "zap",
    dark: "#261004",
    mid: "#ea580c",
    accent: "#facc15",
    soft: "#ffedd5",
    glow: "rgba(250, 204, 21, .36)",
  },
  {
    name: "霞金勋章",
    icon: "medal",
    dark: "#2a1208",
    mid: "#c2410c",
    accent: "#fbbf24",
    soft: "#fed7aa",
    glow: "rgba(251, 191, 36, .34)",
  },
  {
    name: "钛金徽印",
    icon: "badge-check",
    dark: "#1c1608",
    mid: "#a16207",
    accent: "#fef08a",
    soft: "#fff7d6",
    glow: "rgba(254, 240, 138, .34)",
  },
  {
    name: "耀金火焰",
    icon: "flame",
    dark: "#2b0e04",
    mid: "#b45309",
    accent: "#f97316",
    soft: "#ffedd5",
    glow: "rgba(249, 115, 22, .34)",
  },
  {
    name: "币金光环",
    icon: "circle-dollar",
    dark: "#1f1706",
    mid: "#b58900",
    accent: "#f5d76e",
    soft: "#fff8cf",
    glow: "rgba(245, 215, 110, .36)",
  },
  {
    name: "秘金魔杖",
    icon: "wand-sparkles",
    dark: "#1a1207",
    mid: "#a8550f",
    accent: "#fbbf24",
    soft: "#fffbeb",
    glow: "rgba(251, 191, 36, .35)",
  },
  {
    name: "金焰六边",
    icon: "hexagon",
    dark: "#151008",
    mid: "#92400e",
    accent: "#f59e0b",
    soft: "#fef3c7",
    glow: "rgba(245, 158, 11, .34)",
  },
  {
    name: "晨金推进",
    icon: "rocket",
    dark: "#251a08",
    mid: "#ca8a04",
    accent: "#fde047",
    soft: "#fefce8",
    glow: "rgba(253, 224, 71, .35)",
  },
] satisfies Array<{
  name: string;
  icon: CodexPlanBadgeIconKey;
  dark: string;
  mid: string;
  accent: string;
  soft: string;
  glow: string;
}>;

const CODEX_PLAN_TECH_ICON_OFFSET: Record<CodexPlanBadgeTier, number> = {
  free: 0,
  team: 3,
  proLite: 5,
  pro: 6,
  plus: 9,
};

function buildTechBadgeStyles(
  tier: CodexPlanBadgeTier,
): CodexPlanBadgeStyle[] {
  return CODEX_PLAN_TECH_VARIANTS.map((variant, index) => {
    const shiftedIcon =
      CODEX_PLAN_TECH_VARIANTS[
        (index + CODEX_PLAN_TECH_ICON_OFFSET[tier]) %
          CODEX_PLAN_TECH_VARIANTS.length
      ].icon;
    return defineBadgeStyle(
      `style-${index + 11}` as CodexPlanBadgeStyleId,
      `${variant.name}${CODEX_PLAN_TECH_TIER_SUFFIX[tier]}`,
      shiftedIcon,
      {
        bg: `radial-gradient(circle at 18% 10%, ${variant.soft} 0 9%, transparent 24%), linear-gradient(120deg, ${variant.dark} 0%, ${variant.mid} 48%, ${variant.accent} 100%)`,
        color: index === 8 || index === 14 ? "#0f172a" : "#f8fafc",
        border: `color-mix(in srgb, ${variant.accent} 72%, rgba(255,255,255,.28))`,
        shadow: `inset 0 1px 0 rgba(255,255,255,.22), inset 0 -1px 0 rgba(2,6,23,.38), 0 6px 16px ${variant.glow}, 0 0 0 1px rgba(255,255,255,.12)`,
        iconBg: `linear-gradient(145deg, ${variant.soft} 0%, ${variant.accent} 56%, ${variant.mid} 100%)`,
        iconColor: variant.dark,
        iconShadow: `inset 0 1px 0 rgba(255,255,255,.78), 0 0 9px ${variant.glow}, 0 1px 2px rgba(2,6,23,.24)`,
        labelShadow:
          index === 8 || index === 14
            ? "0 1px 0 rgba(255,255,255,.5)"
            : "0 1px 1px rgba(2,6,23,.72), 0 0 8px rgba(255,255,255,.18)",
      },
    );
  });
}

function buildGoldBadgeStyles(): CodexPlanBadgeStyle[] {
  return CODEX_PLAN_GOLD_VARIANTS.map((variant, index) =>
    defineBadgeStyle(
      `style-${index + 26}` as CodexPlanBadgeStyleId,
      variant.name,
      variant.icon,
      {
        bg: `radial-gradient(circle at 18% 10%, ${variant.soft} 0 10%, transparent 26%), linear-gradient(135deg, ${variant.dark} 0%, ${variant.mid} 46%, ${variant.accent} 100%)`,
        color: "#fffbe6",
        border: `color-mix(in srgb, ${variant.accent} 72%, rgba(255,255,255,.34))`,
        shadow: `inset 0 1px 0 rgba(255,255,255,.3), inset 0 -1px 0 rgba(69,26,3,.38), 0 7px 18px ${variant.glow}, 0 0 0 1px rgba(255,248,204,.18)`,
        iconBg: `linear-gradient(145deg, #fffbe6 0%, ${variant.soft} 22%, ${variant.accent} 62%, ${variant.mid} 100%)`,
        iconColor: variant.dark,
        iconShadow: `inset 0 1px 0 rgba(255,255,255,.82), 0 0 10px ${variant.glow}, 0 1px 2px rgba(69,26,3,.24)`,
        labelShadow:
          "0 1px 1px rgba(42,23,5,.78), 0 0 8px rgba(255,232,153,.3)",
      },
    ),
  );
}

function buildPlainTextBadgeStyle(): CodexPlanBadgeStyle {
  return defineBadgeStyle("style-41", "纯文本", null, {
    bg: "linear-gradient(135deg, #ffffff 0%, #f8fafc 48%, #e2e8f0 100%)",
    color: "#111827",
    border: "rgba(148, 163, 184, .62)",
    shadow:
      "inset 0 1px 0 rgba(255,255,255,.86), 0 4px 10px rgba(15,23,42,.1)",
    iconBg: "transparent",
    iconColor: "#111827",
    iconShadow: "none",
    labelShadow: "0 1px 0 rgba(255,255,255,.58)",
  });
}

export const CODEX_PLAN_BADGE_STYLE_SETS: Record<
  CodexPlanBadgeTier,
  CodexPlanBadgeStyle[]
> = {
  free: [
    buildPlainTextBadgeStyle(),
    defineBadgeStyle("style-1", "黑金王冠", "crown", {
      bg: "linear-gradient(180deg, rgba(255,255,255,.18) 0%, transparent 45%), linear-gradient(135deg, #1f2937 0%, #3b2a12 44%, #8a5a08 100%)",
      color: "#fff9df",
      border: "rgba(245, 202, 86, .68)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.24), inset 0 -1px 0 rgba(54,30,4,.42), 0 5px 14px rgba(15,23,42,.16), 0 0 0 1px rgba(255,244,191,.2)",
      iconBg: "linear-gradient(145deg, #fff4c7 0%, #f7cd55 50%, #b87512 100%)",
      iconColor: "#3b2502",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.78), inset 0 -1px 0 rgba(83,46,5,.3), 0 1px 2px rgba(0,0,0,.22)",
      labelShadow: "0 1px 1px rgba(0,0,0,.68), 0 0 5px rgba(255,232,153,.32)",
    }),
    defineBadgeStyle("style-2", "银霜星标", "star", {
      bg: "linear-gradient(135deg, #f8fafc 0%, #dbe4f0 52%, #94a3b8 100%)",
      color: "#1e293b",
      border: "rgba(148, 163, 184, .62)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.78), 0 4px 10px rgba(71,85,105,.12)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #cbd5e1 60%, #64748b 100%)",
      iconColor: "#0f172a",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.8), 0 1px 2px rgba(15,23,42,.2)",
      labelShadow: "0 1px 0 rgba(255,255,255,.46)",
    }),
    defineBadgeStyle("style-3", "蓝钻晶面", "gem", {
      bg: "linear-gradient(135deg, #e0f2fe 0%, #38bdf8 46%, #0f3d64 100%)",
      color: "#f0f9ff",
      border: "rgba(56, 189, 248, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.38), 0 5px 14px rgba(14,116,144,.2)",
      iconBg: "linear-gradient(145deg, #f0f9ff 0%, #7dd3fc 55%, #0369a1 100%)",
      iconColor: "#07344f",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(3,105,161,.24)",
      labelShadow: "0 1px 1px rgba(7,52,79,.62)",
    }),
    defineBadgeStyle("style-4", "薄荷盾牌", "shield-check", {
      bg: "linear-gradient(135deg, #ecfdf5 0%, #86efac 48%, #047857 100%)",
      color: "#052e16",
      border: "rgba(16, 185, 129, .48)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.58), 0 4px 12px rgba(5,150,105,.15)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #a7f3d0 58%, #10b981 100%)",
      iconColor: "#064e3b",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(6,78,59,.18)",
      labelShadow: "0 1px 0 rgba(255,255,255,.38)",
    }),
    defineBadgeStyle("style-5", "琥珀火焰", "flame", {
      bg: "linear-gradient(135deg, #431407 0%, #c2410c 48%, #fed7aa 100%)",
      color: "#fff7ed",
      border: "rgba(251, 146, 60, .62)",
      shadow: "inset 0 1px 0 rgba(255,237,213,.28), 0 5px 14px rgba(194,65,12,.2)",
      iconBg: "linear-gradient(145deg, #ffedd5 0%, #fb923c 52%, #9a3412 100%)",
      iconColor: "#431407",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.7), 0 1px 2px rgba(67,20,7,.26)",
      labelShadow: "0 1px 1px rgba(67,20,7,.7)",
    }),
    defineBadgeStyle("style-6", "电光石墨", "bolt", {
      bg: "linear-gradient(135deg, #111827 0%, #334155 52%, #67e8f9 100%)",
      color: "#ecfeff",
      border: "rgba(103, 232, 249, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.2), 0 5px 14px rgba(8,47,73,.2)",
      iconBg: "linear-gradient(145deg, #ecfeff 0%, #67e8f9 54%, #155e75 100%)",
      iconColor: "#0e2937",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(8,47,73,.22)",
      labelShadow: "0 1px 1px rgba(8,47,73,.72)",
    }),
    defineBadgeStyle("style-7", "青铜勋章", "medal", {
      bg: "linear-gradient(135deg, #fef3c7 0%, #d97706 50%, #78350f 100%)",
      color: "#fff7ed",
      border: "rgba(217, 119, 6, .56)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.4), 0 5px 13px rgba(146,64,14,.18)",
      iconBg: "linear-gradient(145deg, #fff7ed 0%, #f59e0b 55%, #92400e 100%)",
      iconColor: "#451a03",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.76), 0 1px 2px rgba(69,26,3,.22)",
      labelShadow: "0 1px 1px rgba(69,26,3,.62)",
    }),
    defineBadgeStyle("style-8", "珊瑚闪光", "sparkles", {
      bg: "linear-gradient(135deg, #fff1f2 0%, #fb7185 48%, #9f1239 100%)",
      color: "#fff1f2",
      border: "rgba(251, 113, 133, .54)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.48), 0 5px 12px rgba(190,18,60,.16)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #fda4af 58%, #e11d48 100%)",
      iconColor: "#881337",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.76), 0 1px 2px rgba(136,19,55,.2)",
      labelShadow: "0 1px 1px rgba(136,19,55,.62)",
    }),
    defineBadgeStyle("style-9", "紫雾徽记", "badge-check", {
      bg: "linear-gradient(135deg, #f5f3ff 0%, #a78bfa 48%, #4c1d95 100%)",
      color: "#faf5ff",
      border: "rgba(167, 139, 250, .55)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.46), 0 5px 12px rgba(91,33,182,.16)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #c4b5fd 54%, #7c3aed 100%)",
      iconColor: "#3b0764",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(59,7,100,.2)",
      labelShadow: "0 1px 1px rgba(59,7,100,.62)",
    }),
    defineBadgeStyle("style-10", "赛博六边", "hexagon", {
      bg: "linear-gradient(135deg, #0f172a 0%, #155e75 48%, #2dd4bf 100%)",
      color: "#f0fdfa",
      border: "rgba(45, 212, 191, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.2), 0 5px 14px rgba(15,118,110,.2)",
      iconBg: "linear-gradient(145deg, #ccfbf1 0%, #2dd4bf 54%, #0f766e 100%)",
      iconColor: "#042f2e",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.7), 0 1px 2px rgba(4,47,46,.22)",
      labelShadow: "0 1px 1px rgba(4,47,46,.68)",
    }),
    ...buildGoldBadgeStyles(),
    ...buildTechBadgeStyles("free"),
  ],
  team: [
    buildPlainTextBadgeStyle(),
    defineBadgeStyle("style-1", "翡翠小队", "shield-check", {
      bg: "linear-gradient(135deg, #ecfdf5 0%, #34d399 50%, #065f46 100%)",
      color: "#ecfdf5",
      border: "rgba(16, 185, 129, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.38), 0 5px 13px rgba(5,150,105,.18)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #a7f3d0 54%, #10b981 100%)",
      iconColor: "#064e3b",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.72), 0 1px 2px rgba(6,78,59,.2)",
      labelShadow: "0 1px 1px rgba(6,78,59,.68)",
    }),
    defineBadgeStyle("style-2", "靛蓝公会", "badge-check", {
      bg: "linear-gradient(135deg, #eef2ff 0%, #818cf8 48%, #312e81 100%)",
      color: "#eef2ff",
      border: "rgba(129, 140, 248, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.45), 0 5px 13px rgba(67,56,202,.16)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #c7d2fe 55%, #6366f1 100%)",
      iconColor: "#1e1b4b",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(30,27,75,.2)",
      labelShadow: "0 1px 1px rgba(30,27,75,.66)",
    }),
    defineBadgeStyle("style-3", "海湾同盟", "star", {
      bg: "linear-gradient(135deg, #eff6ff 0%, #38bdf8 45%, #1e3a8a 100%)",
      color: "#eff6ff",
      border: "rgba(56, 189, 248, .56)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.42), 0 5px 13px rgba(37,99,235,.16)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #7dd3fc 55%, #2563eb 100%)",
      iconColor: "#172554",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(23,37,84,.2)",
      labelShadow: "0 1px 1px rgba(23,37,84,.66)",
    }),
    defineBadgeStyle("style-4", "极光协作", "sparkles", {
      bg: "linear-gradient(135deg, #ecfeff 0%, #22c55e 45%, #7c3aed 100%)",
      color: "#f8fafc",
      border: "rgba(45, 212, 191, .54)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.38), 0 5px 14px rgba(76,29,149,.16)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #86efac 48%, #a78bfa 100%)",
      iconColor: "#064e3b",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.76), 0 1px 2px rgba(76,29,149,.2)",
      labelShadow: "0 1px 1px rgba(15,23,42,.68)",
    }),
    defineBadgeStyle("style-5", "石墨薄荷", "hexagon", {
      bg: "linear-gradient(135deg, #111827 0%, #334155 50%, #5eead4 100%)",
      color: "#f0fdfa",
      border: "rgba(94, 234, 212, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.2), 0 5px 14px rgba(15,23,42,.18)",
      iconBg: "linear-gradient(145deg, #f0fdfa 0%, #5eead4 55%, #0f766e 100%)",
      iconColor: "#042f2e",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.72), 0 1px 2px rgba(4,47,46,.22)",
      labelShadow: "0 1px 1px rgba(4,47,46,.7)",
    }),
    defineBadgeStyle("style-6", "柠檬作战", "zap", {
      bg: "linear-gradient(135deg, #f7fee7 0%, #a3e635 48%, #3f6212 100%)",
      color: "#1a2e05",
      border: "rgba(132, 204, 22, .52)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.5), 0 5px 12px rgba(77,124,15,.15)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #d9f99d 55%, #84cc16 100%)",
      iconColor: "#365314",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(54,83,20,.18)",
      labelShadow: "0 1px 0 rgba(255,255,255,.42)",
    }),
    defineBadgeStyle("style-7", "赤焰指挥", "flame", {
      bg: "linear-gradient(135deg, #fff1f2 0%, #f43f5e 48%, #7f1d1d 100%)",
      color: "#fff1f2",
      border: "rgba(244, 63, 94, .56)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.42), 0 5px 13px rgba(159,18,57,.16)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #fda4af 55%, #e11d48 100%)",
      iconColor: "#7f1d1d",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(127,29,29,.2)",
      labelShadow: "0 1px 1px rgba(127,29,29,.66)",
    }),
    defineBadgeStyle("style-8", "紫晶小组", "gem", {
      bg: "linear-gradient(135deg, #faf5ff 0%, #c084fc 48%, #581c87 100%)",
      color: "#faf5ff",
      border: "rgba(192, 132, 252, .56)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.44), 0 5px 13px rgba(126,34,206,.15)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #e9d5ff 55%, #a855f7 100%)",
      iconColor: "#3b0764",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(59,7,100,.18)",
      labelShadow: "0 1px 1px rgba(59,7,100,.62)",
    }),
    defineBadgeStyle("style-9", "冰川舰队", "rocket", {
      bg: "linear-gradient(135deg, #f0f9ff 0%, #93c5fd 48%, #0f172a 100%)",
      color: "#f8fafc",
      border: "rgba(147, 197, 253, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.45), 0 5px 13px rgba(30,64,175,.15)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #bfdbfe 55%, #3b82f6 100%)",
      iconColor: "#172554",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(23,37,84,.2)",
      labelShadow: "0 1px 1px rgba(15,23,42,.66)",
    }),
    defineBadgeStyle("style-10", "金色议会", "trophy", {
      bg: "linear-gradient(135deg, #fffbeb 0%, #fbbf24 48%, #713f12 100%)",
      color: "#fff7ed",
      border: "rgba(251, 191, 36, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.48), 0 5px 13px rgba(180,83,9,.17)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #fde68a 55%, #d97706 100%)",
      iconColor: "#451a03",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(69,26,3,.2)",
      labelShadow: "0 1px 1px rgba(69,26,3,.64)",
    }),
    ...buildGoldBadgeStyles(),
    ...buildTechBadgeStyles("team"),
  ],
  proLite: [
    buildPlainTextBadgeStyle(),
    defineBadgeStyle("style-1", "轻核紫钻", "gem", {
      bg: "linear-gradient(135deg, #f5f3ff 0%, #a78bfa 48%, #4c1d95 100%)",
      color: "#faf5ff",
      border: "rgba(167, 139, 250, .6)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.46), 0 5px 14px rgba(91,33,182,.18)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #ddd6fe 55%, #8b5cf6 100%)",
      iconColor: "#2e1065",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.76), 0 1px 2px rgba(46,16,101,.2)",
      labelShadow: "0 1px 1px rgba(46,16,101,.66)",
    }),
    defineBadgeStyle("style-2", "星翼火箭", "rocket", {
      bg: "linear-gradient(135deg, #eff6ff 0%, #60a5fa 48%, #1e3a8a 100%)",
      color: "#eff6ff",
      border: "rgba(96, 165, 250, .6)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.42), 0 5px 14px rgba(37,99,235,.17)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #bfdbfe 55%, #3b82f6 100%)",
      iconColor: "#172554",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.76), 0 1px 2px rgba(23,37,84,.2)",
      labelShadow: "0 1px 1px rgba(23,37,84,.66)",
    }),
    defineBadgeStyle("style-3", "钛银星徽", "star", {
      bg: "linear-gradient(135deg, #ffffff 0%, #dbeafe 40%, #64748b 100%)",
      color: "#111827",
      border: "rgba(148, 163, 184, .62)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.84), 0 5px 13px rgba(71,85,105,.14)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #e2e8f0 55%, #94a3b8 100%)",
      iconColor: "#0f172a",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.82), 0 1px 2px rgba(15,23,42,.18)",
      labelShadow: "0 1px 0 rgba(255,255,255,.5)",
    }),
    defineBadgeStyle("style-4", "青金脉冲", "bolt", {
      bg: "linear-gradient(135deg, #ecfdf5 0%, #2dd4bf 46%, #0f766e 100%)",
      color: "#f0fdfa",
      border: "rgba(45, 212, 191, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.36), 0 5px 14px rgba(15,118,110,.18)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #99f6e4 55%, #14b8a6 100%)",
      iconColor: "#042f2e",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(4,47,46,.2)",
      labelShadow: "0 1px 1px rgba(4,47,46,.66)",
    }),
    defineBadgeStyle("style-5", "赤羽火焰", "flame", {
      bg: "linear-gradient(135deg, #fff1f2 0%, #fb7185 47%, #7f1d1d 100%)",
      color: "#fff1f2",
      border: "rgba(251, 113, 133, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.42), 0 5px 13px rgba(159,18,57,.16)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #fecdd3 55%, #f43f5e 100%)",
      iconColor: "#7f1d1d",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(127,29,29,.2)",
      labelShadow: "0 1px 1px rgba(127,29,29,.64)",
    }),
    defineBadgeStyle("style-6", "蓝弧奖杯", "trophy", {
      bg: "linear-gradient(135deg, #e0f2fe 0%, #38bdf8 48%, #075985 100%)",
      color: "#f0f9ff",
      border: "rgba(56, 189, 248, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.4), 0 5px 13px rgba(14,116,144,.16)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #bae6fd 55%, #0ea5e9 100%)",
      iconColor: "#083344",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(8,51,68,.2)",
      labelShadow: "0 1px 1px rgba(8,51,68,.64)",
    }),
    defineBadgeStyle("style-7", "琥珀轻勋", "medal", {
      bg: "linear-gradient(135deg, #fffbeb 0%, #fbbf24 48%, #92400e 100%)",
      color: "#fff7ed",
      border: "rgba(251, 191, 36, .6)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.44), 0 5px 13px rgba(180,83,9,.17)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #fde68a 55%, #d97706 100%)",
      iconColor: "#451a03",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.76), 0 1px 2px rgba(69,26,3,.2)",
      labelShadow: "0 1px 1px rgba(69,26,3,.64)",
    }),
    defineBadgeStyle("style-8", "晶格盾章", "shield-check", {
      bg: "linear-gradient(135deg, #f0fdfa 0%, #34d399 48%, #064e3b 100%)",
      color: "#ecfdf5",
      border: "rgba(52, 211, 153, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.38), 0 5px 13px rgba(5,150,105,.18)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #a7f3d0 55%, #10b981 100%)",
      iconColor: "#064e3b",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(6,78,59,.2)",
      labelShadow: "0 1px 1px rgba(6,78,59,.66)",
    }),
    defineBadgeStyle("style-9", "黑曜轻翼", "diamond", {
      bg: "linear-gradient(135deg, #020617 0%, #334155 48%, #cbd5e1 100%)",
      color: "#f8fafc",
      border: "rgba(203, 213, 225, .56)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.2), 0 5px 14px rgba(15,23,42,.22)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #cbd5e1 55%, #475569 100%)",
      iconColor: "#020617",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.76), 0 1px 2px rgba(2,6,23,.24)",
      labelShadow: "0 1px 1px rgba(2,6,23,.72)",
    }),
    defineBadgeStyle("style-10", "霓光徽记", "badge-check", {
      bg: "linear-gradient(135deg, #faf5ff 0%, #22d3ee 44%, #6d28d9 100%)",
      color: "#f8fafc",
      border: "rgba(34, 211, 238, .56)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.36), 0 5px 14px rgba(76,29,149,.16)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #a5f3fc 48%, #a78bfa 100%)",
      iconColor: "#312e81",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.76), 0 1px 2px rgba(49,46,129,.2)",
      labelShadow: "0 1px 1px rgba(49,46,129,.66)",
    }),
    ...buildGoldBadgeStyles(),
    ...buildTechBadgeStyles("proLite"),
  ],
  pro: [
    buildPlainTextBadgeStyle(),
    defineBadgeStyle("style-1", "皇家晶核", "gem", {
      bg: "linear-gradient(135deg, #f5f3ff 0%, #8b5cf6 48%, #2e1065 100%)",
      color: "#faf5ff",
      border: "rgba(139, 92, 246, .6)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.42), 0 5px 14px rgba(91,33,182,.18)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #ddd6fe 54%, #7c3aed 100%)",
      iconColor: "#2e1065",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.76), 0 1px 2px rgba(46,16,101,.22)",
      labelShadow: "0 1px 1px rgba(46,16,101,.68)",
    }),
    defineBadgeStyle("style-2", "钴蓝火箭", "rocket", {
      bg: "linear-gradient(135deg, #eff6ff 0%, #3b82f6 48%, #1e1b4b 100%)",
      color: "#eff6ff",
      border: "rgba(59, 130, 246, .6)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.42), 0 5px 14px rgba(29,78,216,.18)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #bfdbfe 55%, #2563eb 100%)",
      iconColor: "#172554",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(23,37,84,.22)",
      labelShadow: "0 1px 1px rgba(23,37,84,.66)",
    }),
    defineBadgeStyle("style-3", "碳纤钻石", "diamond", {
      bg: "linear-gradient(135deg, #020617 0%, #334155 48%, #e2e8f0 100%)",
      color: "#f8fafc",
      border: "rgba(203, 213, 225, .54)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.18), 0 5px 14px rgba(15,23,42,.22)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #cbd5e1 55%, #475569 100%)",
      iconColor: "#020617",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.76), 0 1px 2px rgba(2,6,23,.24)",
      labelShadow: "0 1px 1px rgba(2,6,23,.72)",
    }),
    defineBadgeStyle("style-4", "霓虹脉冲", "bolt", {
      bg: "linear-gradient(135deg, #022c22 0%, #10b981 48%, #d9f99d 100%)",
      color: "#ecfdf5",
      border: "rgba(52, 211, 153, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.3), 0 5px 14px rgba(5,150,105,.2)",
      iconBg: "linear-gradient(145deg, #ecfdf5 0%, #6ee7b7 55%, #059669 100%)",
      iconColor: "#022c22",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(2,44,34,.22)",
      labelShadow: "0 1px 1px rgba(2,44,34,.7)",
    }),
    defineBadgeStyle("style-5", "绯红引擎", "flame", {
      bg: "linear-gradient(135deg, #450a0a 0%, #dc2626 48%, #fecaca 100%)",
      color: "#fef2f2",
      border: "rgba(248, 113, 113, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.32), 0 5px 14px rgba(185,28,28,.18)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #fca5a5 55%, #dc2626 100%)",
      iconColor: "#450a0a",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.76), 0 1px 2px rgba(69,10,10,.22)",
      labelShadow: "0 1px 1px rgba(69,10,10,.68)",
    }),
    defineBadgeStyle("style-6", "蓝宝奖杯", "trophy", {
      bg: "linear-gradient(135deg, #e0f2fe 0%, #0284c7 48%, #082f49 100%)",
      color: "#f0f9ff",
      border: "rgba(14, 165, 233, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.38), 0 5px 14px rgba(2,132,199,.18)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #7dd3fc 55%, #0ea5e9 100%)",
      iconColor: "#082f49",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.76), 0 1px 2px rgba(8,47,73,.22)",
      labelShadow: "0 1px 1px rgba(8,47,73,.68)",
    }),
    defineBadgeStyle("style-7", "白金星轨", "star", {
      bg: "linear-gradient(135deg, #ffffff 0%, #cbd5e1 44%, #64748b 100%)",
      color: "#111827",
      border: "rgba(148, 163, 184, .62)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.82), 0 5px 13px rgba(71,85,105,.14)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #e2e8f0 55%, #94a3b8 100%)",
      iconColor: "#0f172a",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.82), 0 1px 2px rgba(15,23,42,.18)",
      labelShadow: "0 1px 0 rgba(255,255,255,.5)",
    }),
    defineBadgeStyle("style-8", "琥珀专家", "medal", {
      bg: "linear-gradient(135deg, #fffbeb 0%, #f59e0b 48%, #78350f 100%)",
      color: "#fff7ed",
      border: "rgba(245, 158, 11, .6)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.42), 0 5px 13px rgba(180,83,9,.17)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #fde68a 55%, #d97706 100%)",
      iconColor: "#451a03",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.76), 0 1px 2px rgba(69,26,3,.2)",
      labelShadow: "0 1px 1px rgba(69,26,3,.64)",
    }),
    defineBadgeStyle("style-9", "青焰闪耀", "sparkles", {
      bg: "linear-gradient(135deg, #f0fdfa 0%, #14b8a6 48%, #134e4a 100%)",
      color: "#f0fdfa",
      border: "rgba(20, 184, 166, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.38), 0 5px 13px rgba(15,118,110,.18)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #99f6e4 55%, #14b8a6 100%)",
      iconColor: "#042f2e",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(4,47,46,.2)",
      labelShadow: "0 1px 1px rgba(4,47,46,.66)",
    }),
    defineBadgeStyle("style-10", "黑曜棱镜", "hexagon", {
      bg: "linear-gradient(135deg, #020617 0%, #4c1d95 45%, #facc15 100%)",
      color: "#fefce8",
      border: "rgba(250, 204, 21, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.2), 0 5px 14px rgba(76,29,149,.18)",
      iconBg: "linear-gradient(145deg, #fefce8 0%, #c084fc 45%, #facc15 100%)",
      iconColor: "#2e1065",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(46,16,101,.22)",
      labelShadow: "0 1px 1px rgba(15,23,42,.74)",
    }),
    ...buildGoldBadgeStyles(),
    ...buildTechBadgeStyles("pro"),
  ],
  plus: [
    buildPlainTextBadgeStyle(),
    defineBadgeStyle("style-1", "薄荷加冕", "sparkles", {
      bg: "linear-gradient(135deg, #ecfdf5 0%, #5eead4 50%, #047857 100%)",
      color: "#052e16",
      border: "rgba(45, 212, 191, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.56), 0 5px 13px rgba(13,148,136,.16)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #ccfbf1 55%, #14b8a6 100%)",
      iconColor: "#064e3b",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.76), 0 1px 2px rgba(6,78,59,.18)",
      labelShadow: "0 1px 0 rgba(255,255,255,.42)",
    }),
    defineBadgeStyle("style-2", "海潮闪电", "bolt", {
      bg: "linear-gradient(135deg, #eff6ff 0%, #38bdf8 48%, #0f766e 100%)",
      color: "#f0fdfa",
      border: "rgba(56, 189, 248, .56)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.4), 0 5px 13px rgba(14,116,144,.16)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #bae6fd 55%, #06b6d4 100%)",
      iconColor: "#083344",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(8,51,68,.2)",
      labelShadow: "0 1px 1px rgba(8,51,68,.62)",
    }),
    defineBadgeStyle("style-3", "金冠增幅", "crown", {
      bg: "linear-gradient(135deg, #fffbeb 0%, #fbbf24 48%, #854d0e 100%)",
      color: "#fff7ed",
      border: "rgba(251, 191, 36, .62)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.48), 0 5px 13px rgba(180,83,9,.17)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #fde68a 55%, #d97706 100%)",
      iconColor: "#451a03",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.76), 0 1px 2px rgba(69,26,3,.2)",
      labelShadow: "0 1px 1px rgba(69,26,3,.64)",
    }),
    defineBadgeStyle("style-4", "玫瑰钻面", "diamond", {
      bg: "linear-gradient(135deg, #fff1f2 0%, #f472b6 48%, #831843 100%)",
      color: "#fff1f2",
      border: "rgba(244, 114, 182, .56)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.42), 0 5px 13px rgba(190,24,93,.16)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #fbcfe8 55%, #ec4899 100%)",
      iconColor: "#831843",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(131,24,67,.2)",
      labelShadow: "0 1px 1px rgba(131,24,67,.64)",
    }),
    defineBadgeStyle("style-5", "青空推进", "rocket", {
      bg: "linear-gradient(135deg, #ecfeff 0%, #22d3ee 48%, #164e63 100%)",
      color: "#ecfeff",
      border: "rgba(34, 211, 238, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.42), 0 5px 13px rgba(14,116,144,.16)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #a5f3fc 55%, #06b6d4 100%)",
      iconColor: "#083344",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(8,51,68,.2)",
      labelShadow: "0 1px 1px rgba(8,51,68,.64)",
    }),
    defineBadgeStyle("style-6", "青柠徽章", "badge", {
      bg: "linear-gradient(135deg, #f7fee7 0%, #bef264 48%, #365314 100%)",
      color: "#1a2e05",
      border: "rgba(163, 230, 53, .56)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.54), 0 5px 12px rgba(77,124,15,.14)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #d9f99d 55%, #84cc16 100%)",
      iconColor: "#365314",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(54,83,20,.18)",
      labelShadow: "0 1px 0 rgba(255,255,255,.42)",
    }),
    defineBadgeStyle("style-7", "紫光魔杖", "wand-sparkles", {
      bg: "linear-gradient(135deg, #faf5ff 0%, #a78bfa 48%, #4c1d95 100%)",
      color: "#faf5ff",
      border: "rgba(167, 139, 250, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.42), 0 5px 13px rgba(91,33,182,.16)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #ddd6fe 55%, #8b5cf6 100%)",
      iconColor: "#3b0764",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(59,7,100,.2)",
      labelShadow: "0 1px 1px rgba(59,7,100,.62)",
    }),
    defineBadgeStyle("style-8", "钢蓝奖杯", "trophy", {
      bg: "linear-gradient(135deg, #f8fafc 0%, #94a3b8 48%, #1e3a8a 100%)",
      color: "#f8fafc",
      border: "rgba(148, 163, 184, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.42), 0 5px 13px rgba(30,64,175,.14)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #cbd5e1 55%, #64748b 100%)",
      iconColor: "#0f172a",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(15,23,42,.18)",
      labelShadow: "0 1px 1px rgba(15,23,42,.64)",
    }),
    defineBadgeStyle("style-9", "橙焰加速", "flame", {
      bg: "linear-gradient(135deg, #fff7ed 0%, #fb923c 48%, #7c2d12 100%)",
      color: "#fff7ed",
      border: "rgba(251, 146, 60, .58)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.42), 0 5px 13px rgba(194,65,12,.16)",
      iconBg: "linear-gradient(145deg, #ffffff 0%, #fed7aa 55%, #f97316 100%)",
      iconColor: "#431407",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(67,20,7,.2)",
      labelShadow: "0 1px 1px rgba(67,20,7,.64)",
    }),
    defineBadgeStyle("style-10", "黑绿宝石", "gem", {
      bg: "linear-gradient(135deg, #020617 0%, #064e3b 46%, #a7f3d0 100%)",
      color: "#ecfdf5",
      border: "rgba(167, 243, 208, .56)",
      shadow: "inset 0 1px 0 rgba(255,255,255,.2), 0 5px 14px rgba(6,78,59,.18)",
      iconBg: "linear-gradient(145deg, #ecfdf5 0%, #6ee7b7 55%, #059669 100%)",
      iconColor: "#022c22",
      iconShadow: "inset 0 1px 0 rgba(255,255,255,.74), 0 1px 2px rgba(2,44,34,.22)",
      labelShadow: "0 1px 1px rgba(2,44,34,.7)",
    }),
    ...buildGoldBadgeStyles(),
    ...buildTechBadgeStyles("plus"),
  ],
};

function isKnownStyleId(
  tier: CodexPlanBadgeTier,
  value: unknown,
): value is CodexPlanBadgeStyleId {
  return (
    typeof value === "string" &&
    CODEX_PLAN_BADGE_STYLE_SETS[tier].some((style) => style.id === value)
  );
}

function buildPlanBadgeStyleVars(
  style: CodexPlanBadgeStyle,
): CSSProperties & Record<string, string> {
  return {
    "--codex-plan-badge-bg": style.theme.bg,
    "--codex-plan-badge-color": style.theme.color,
    "--codex-plan-badge-border": style.theme.border,
    "--codex-plan-badge-shadow": style.theme.shadow,
    "--codex-plan-badge-icon-bg": style.theme.iconBg,
    "--codex-plan-badge-icon-color": style.theme.iconColor,
    "--codex-plan-badge-icon-shadow": style.theme.iconShadow,
    "--codex-plan-badge-label-shadow": style.theme.labelShadow,
  };
}

export function resolveCodexPlanBadgeTier(
  planClass: string | null | undefined,
  planLabel: string | null | undefined,
): CodexPlanBadgeTier | null {
  const normalized = `${planClass || ""} ${planLabel || ""}`.toLowerCase();
  if (!normalized.trim()) return null;
  if (normalized.includes("api-key") || normalized.includes("new-api")) {
    return null;
  }
  if (normalized.includes("free")) return "free";
  if (normalized.includes("plus") && !normalized.includes("pro-plus")) {
    return "plus";
  }
  if (
    normalized.includes("codex-pro-lite") ||
    normalized.includes("pro-lite") ||
    normalized.includes("prolite") ||
    normalized.includes("pro 5x") ||
    normalized.includes("pro-5x") ||
    normalized.includes("pro_5x")
  ) {
    return "proLite";
  }
  if (
    normalized.includes("pro") ||
    normalized.includes("ultra") ||
    normalized.includes("ultimate")
  ) {
    return "pro";
  }
  if (
    normalized.includes("team") ||
    normalized.includes("enterprise") ||
    normalized.includes("business") ||
    normalized.includes("edu")
  ) {
    return "team";
  }
  return null;
}

export function getCodexPlanBadgeStyle(
  tier: CodexPlanBadgeTier,
  preferences: CodexPlanBadgeStylePreferences,
): CodexPlanBadgeStyle {
  const styleId = isKnownStyleId(tier, preferences[tier])
    ? preferences[tier]
    : DEFAULT_CODEX_PLAN_BADGE_STYLE_PREFERENCES[tier];
  return (
    CODEX_PLAN_BADGE_STYLE_SETS[tier].find((style) => style.id === styleId) ??
    CODEX_PLAN_BADGE_STYLE_SETS[tier][0]
  );
}

export function readCodexPlanBadgeStylePreferences(): CodexPlanBadgeStylePreferences {
  if (typeof window === "undefined") {
    return { ...DEFAULT_CODEX_PLAN_BADGE_STYLE_PREFERENCES };
  }
  try {
    const raw = window.localStorage.getItem(CODEX_PLAN_BADGE_STYLE_STORAGE_KEY);
    const parsed = raw ? JSON.parse(raw) : null;
    const next = { ...DEFAULT_CODEX_PLAN_BADGE_STYLE_PREFERENCES };
    if (parsed && typeof parsed === "object") {
      CODEX_PLAN_BADGE_TIERS.forEach((tier) => {
        const value = (parsed as Record<string, unknown>)[tier];
        if (isKnownStyleId(tier, value)) {
          next[tier] = value;
        }
      });
    }
    return next;
  } catch {
    return { ...DEFAULT_CODEX_PLAN_BADGE_STYLE_PREFERENCES };
  }
}

export function writeCodexPlanBadgeStylePreferences(
  preferences: CodexPlanBadgeStylePreferences,
) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(
      CODEX_PLAN_BADGE_STYLE_STORAGE_KEY,
      JSON.stringify(preferences),
    );
  } catch {
    // localStorage may be unavailable in hardened webviews.
  }
}

interface CodexPlanBadgeProps {
  planClass: string | null | undefined;
  planLabel: string;
  preferences: CodexPlanBadgeStylePreferences;
  extraClassName?: string;
}

export function CodexPlanBadge({
  planClass,
  planLabel,
  preferences,
  extraClassName = "",
}: CodexPlanBadgeProps) {
  const normalizedClass = planClass || "unknown";
  const tier = resolveCodexPlanBadgeTier(normalizedClass, planLabel);
  const style = tier ? getCodexPlanBadgeStyle(tier, preferences) : null;
  const Icon = style?.icon ? CODEX_PLAN_BADGE_ICONS[style.icon] : null;
  const className = [
    "tier-badge",
    normalizedClass,
    style ? "codex-plan-badge-custom" : "",
    style && !Icon ? "codex-plan-badge-no-icon" : "",
    tier ? `codex-plan-badge-${tier}` : "",
    style ? `codex-plan-style-${style.id}` : "",
    extraClassName,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <span
      className={className}
      title={planLabel}
      aria-label={planLabel}
      style={style ? buildPlanBadgeStyleVars(style) : undefined}
    >
      {style ? (
        <>
          {Icon ? (
            <span className="codex-plan-icon-shell" aria-hidden="true">
              <Icon size={12} strokeWidth={2.65} />
            </span>
          ) : null}
          <span className="codex-plan-label">{planLabel}</span>
        </>
      ) : (
        planLabel
      )}
    </span>
  );
}
