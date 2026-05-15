import { Palette, RotateCw, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  CODEX_PLAN_BADGE_STYLE_SETS,
  CODEX_PLAN_BADGE_TIER_META,
  CODEX_PLAN_BADGE_TIERS,
  CodexPlanBadge,
  type CodexPlanBadgeStyleId,
  type CodexPlanBadgeStylePreferences,
  type CodexPlanBadgeTier,
} from "./CodexPlanBadge";

interface CodexPlanBadgeStyleModalProps {
  preferences: CodexPlanBadgeStylePreferences;
  onChange: (tier: CodexPlanBadgeTier, styleId: CodexPlanBadgeStyleId) => void;
  onReset: () => void;
  onClose: () => void;
}

export function CodexPlanBadgeStyleModal({
  preferences,
  onChange,
  onReset,
  onClose,
}: CodexPlanBadgeStyleModalProps) {
  const { t } = useTranslation();

  return (
    <div className="modal-overlay codex-plan-style-overlay" onClick={onClose}>
      <div
        className="modal codex-plan-style-modal"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="modal-header codex-plan-style-modal-header">
          <div className="codex-plan-style-title">
            <span className="codex-plan-style-title-icon" aria-hidden="true">
              <Palette size={18} />
            </span>
            <h2>{t("codex.badgeStyle.title", "徽章图标样式")}</h2>
          </div>
          <button
            className="modal-close"
            onClick={onClose}
            aria-label={t("common.close", "关闭")}
          >
            <X />
          </button>
        </div>

        <div className="modal-body codex-plan-style-body">
          {CODEX_PLAN_BADGE_TIERS.map((tier) => {
            const meta = CODEX_PLAN_BADGE_TIER_META[tier];
            const selectedStyleId = preferences[tier];
            return (
              <section key={tier} className="codex-plan-style-section">
                <div className="codex-plan-style-section-header">
                  <div className="codex-plan-style-section-title">
                    <span>{meta.label}</span>
                    <CodexPlanBadge
                      planClass={meta.planClass}
                      planLabel={meta.previewLabel}
                      preferences={preferences}
                    />
                  </div>
                </div>
                <div className="codex-plan-style-grid">
                  {CODEX_PLAN_BADGE_STYLE_SETS[tier].map((style) => {
                    const selected = selectedStyleId === style.id;
                    const previewPreferences = {
                      ...preferences,
                      [tier]: style.id,
                    };
                    const styleName = t(
                      `codex.badgeStyle.styles.${tier}.${style.id}`,
                      style.name,
                    );
                    return (
                      <button
                        key={style.id}
                        type="button"
                        className={`codex-plan-style-option ${selected ? "selected" : ""}`}
                        onClick={() => onChange(tier, style.id)}
                        aria-pressed={selected}
                        title={styleName}
                      >
                        <CodexPlanBadge
                          planClass={meta.planClass}
                          planLabel={meta.previewLabel}
                          preferences={previewPreferences}
                        />
                        <span className="codex-plan-style-option-name">
                          {styleName}
                        </span>
                      </button>
                    );
                  })}
                </div>
              </section>
            );
          })}
        </div>

        <div className="modal-footer codex-plan-style-footer">
          <button className="btn btn-secondary" onClick={onReset}>
            <RotateCw size={14} />
            {t("codex.badgeStyle.reset", "恢复默认")}
          </button>
          <button className="btn btn-primary" onClick={onClose}>
            {t("common.confirm", "确认")}
          </button>
        </div>
      </div>
    </div>
  );
}
