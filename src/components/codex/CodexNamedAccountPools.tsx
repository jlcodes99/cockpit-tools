import { useState } from "react";
import { Plus } from "lucide-react";
import { useTranslation } from "react-i18next";
import { SingleSelectDropdown } from "../SingleSelectDropdown";
import type { CodexAccount } from "../../types/codex";
import type { CodexLocalAccessAccountPool } from "../../types/codexLocalAccess";
import { selectCodexApiKeyScopeAccounts } from "../../utils/codexApiKeyAccountScope";
import { buildCodexAccountPresentation } from "../../presentation/platformAccountPresentation";
import "./CodexNamedAccountPools.css";

interface Props {
  pools: CodexLocalAccessAccountPool[];
  accounts: CodexAccount[];
  busy: boolean;
  maskAccountText: (value: string) => string;
  onCreateKey: (poolId?: string) => Promise<void>;
  onSave: (poolId: string | null, label: string, accountIds: string[]) => Promise<boolean>;
  onRemove: (poolId: string) => Promise<boolean>;
}

type Draft = { id: string | null; label: string; accountIds: string[] };

export function CodexNamedAccountPools({
  pools, accounts, busy, maskAccountText, onCreateKey, onSave, onRemove,
}: Props) {
  const { t } = useTranslation();
  const [poolId, setPoolId] = useState("");
  const [draft, setDraft] = useState<Draft | null>(null);
  const [removePoolId, setRemovePoolId] = useState<string | null>(null);
  const selectedPool = pools.find((pool) => pool.id === poolId);
  const poolLabel = t("codex.apiService.accountPools.title");
  const selectableAccounts = draft ? selectCodexApiKeyScopeAccounts({
    accounts, scopedAccountIds: draft.accountIds,
  }) : [];

  return (
    <div className="codex-named-pools">
      <div className="codex-named-pools-actions">
        <SingleSelectDropdown
          ariaLabel={poolLabel}
          value={poolId}
          options={[
            { value: "", label: t("codex.apiService.keys.accountScopeModeInherit") },
            ...pools.map((pool) => ({ value: pool.id, label: pool.label })),
          ]}
          onChange={setPoolId}
          placeholder={t("common.none")}
          disabled={busy}
        />
        <button type="button" className="btn btn-primary btn-sm"
          onClick={() => void onCreateKey(selectedPool?.id)}
          disabled={busy || Boolean(poolId && !selectedPool)}>
          <Plus size={14} />
          {t("codex.localAccess.apiKeyAdd")}
        </button>
      </div>

      <details className="codex-named-pools-manager">
        <summary>{poolLabel}</summary>
        <div className="codex-named-pools-editor">
          {pools.map((pool) => (
            <div className="codex-named-pools-row" key={pool.id}>
              <span>
                <strong>{pool.label}</strong>
                <small>{t("codex.apiService.keys.accountScopeSelected", { count: pool.accountIds.length })}</small>
              </span>
              <button type="button" className="btn btn-secondary btn-sm" disabled={busy}
                onClick={() => { setRemovePoolId(null); setDraft(pool); }}>
                {t("common.edit")}
              </button>
              <button type="button" className="btn btn-secondary btn-sm" disabled={busy}
                onClick={() => setRemovePoolId(pool.id)}>
                {t("common.delete")}
              </button>
              {removePoolId === pool.id && (
                <div className="codex-named-pools-remove">
                  <p>{t("codex.apiService.accountPools.deleteHint")}</p>
                  <button type="button" className="btn btn-secondary btn-sm" disabled={busy}
                    onClick={() => setRemovePoolId(null)}>{t("common.cancel")}</button>
                  <button type="button" className="btn btn-danger btn-sm" disabled={busy}
                    onClick={async () => {
                      if (!await onRemove(pool.id)) return;
                      setRemovePoolId(null);
                      if (draft?.id === pool.id) setDraft(null);
                      if (poolId === pool.id) setPoolId("");
                    }}>{t("common.confirm")}</button>
                </div>
              )}
            </div>
          ))}
          {draft ? (
            <form className="codex-named-pools-form" onSubmit={async (event) => {
              event.preventDefault();
              if (busy || !draft.label.trim()) return;
              if (await onSave(draft.id, draft.label.trim(), draft.accountIds)) setDraft(null);
            }}>
              <label>
                {t("codex.apiService.accountPools.name")}
                <input className="form-input" value={draft.label} required disabled={busy}
                  onChange={(event) => setDraft({ ...draft, label: event.target.value })} />
              </label>
              <div className="codex-named-pools-members">
                {selectableAccounts.map((account) => {
                  const presentation = buildCodexAccountPresentation(account, t);
                  return (
                    <label key={account.id} className="codex-named-pools-member">
                      <input type="checkbox" checked={draft.accountIds.includes(account.id)} disabled={busy}
                        onChange={(event) => {
                          const checked = event.target.checked;
                          setDraft((current) => current && ({
                            ...current,
                            accountIds: checked ? [...current.accountIds, account.id]
                              : current.accountIds.filter((id) => id !== account.id),
                          }));
                        }}
                      />
                      <span>{maskAccountText(presentation.displayName)}</span>
                      <small>{presentation.planLabel}</small>
                    </label>
                  );
                })}
                {selectableAccounts.length === 0 && <p>{t("codex.localAccess.emptyMembers")}</p>}
              </div>
              <div className="codex-named-pools-actions">
                <button type="button" className="btn btn-secondary btn-sm" disabled={busy}
                  onClick={() => setDraft(null)}>{t("common.cancel")}</button>
                <button type="submit" className="btn btn-primary btn-sm" disabled={busy || !draft.label.trim()}>
                  {t("common.save")}
                </button>
              </div>
            </form>
          ) : (
            <button type="button" className="btn btn-secondary btn-sm" disabled={busy}
              onClick={() => { setRemovePoolId(null); setDraft({ id: null, label: "", accountIds: [] }); }}>
              <Plus size={14} />{t("codex.apiService.accountPools.add")}
            </button>
          )}
        </div>
      </details>
    </div>
  );
}
