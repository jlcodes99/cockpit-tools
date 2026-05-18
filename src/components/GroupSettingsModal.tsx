/**
 * 分组管理弹窗组件
 * 固定分组 + 其他分组（显示黑名单外全部模型）
 */

import React, { useState, useEffect, useCallback, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { X, Package } from 'lucide-react';
import {
  getGroupSettings,
  saveGroupSettings,
} from '../services/groupService';
import { useEscClose } from '../hooks/useEscClose';
import {
  getModelDisplayName,
  getDefaultGroups,
  isBlacklistedModel,
  resolveDefaultGroupId,
} from '../utils/modelNames';
import './GroupSettingsModal.css';

interface GroupSettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
  availableModels?: Array<{
    id: string;
    displayName?: string;
  }>;
}

interface GroupData {
  id: string;
  name: string;
  models: string[];
}

export const GroupSettingsModal: React.FC<GroupSettingsModalProps> = ({
  isOpen,
  onClose,
  availableModels = [],
}) => {
  const { t } = useTranslation();
  useEscClose(isOpen, onClose);
  const [groups, setGroups] = useState<GroupData[]>([]);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const visibleModels = useMemo(() => {
    const modelMap = new Map<string, string | undefined>();
    for (const model of availableModels) {
      const modelId = model.id?.trim();
      if (!modelId) {
        continue;
      }
      const displayName = model.displayName?.trim() || undefined;
      if (isBlacklistedModel(modelId, displayName)) {
        continue;
      }
      const existing = modelMap.get(modelId);
      if (!existing && displayName) {
        modelMap.set(modelId, displayName);
      } else if (!modelMap.has(modelId)) {
        modelMap.set(modelId, undefined);
      }
    }
    return Array.from(modelMap.entries()).map(([id, displayName]) => ({
      id,
      displayName,
    }));
  }, [availableModels]);

  const modelDisplayNameMap = useMemo(() => {
    const map = new Map<string, string>();
    for (const model of visibleModels) {
      if (model.displayName) {
        map.set(model.id, model.displayName);
      }
    }
    return map;
  }, [visibleModels]);

  const buildGroups = useCallback((savedGroupNames?: Record<string, string>): GroupData[] => {
    const modelDisplayMap = new Map<string, string | undefined>();
    for (const model of visibleModels) {
      modelDisplayMap.set(model.id, model.displayName);
    }

    const groupedModelSets = new Map<string, Set<string>>();
    const defaultGroups = getDefaultGroups().map(group => {
      groupedModelSets.set(group.id, new Set<string>());
      return {
        id: group.id,
        name: group.name,
        models: [] as string[],
      };
    });

    for (const model of visibleModels) {
      const matchedGroupId = resolveDefaultGroupId(model.id, model.displayName);
      if (!matchedGroupId) {
        continue;
      }
      groupedModelSets.get(matchedGroupId)?.add(model.id);
    }

    for (const group of defaultGroups) {
      group.models = Array.from(groupedModelSets.get(group.id) || []).sort((a, b) =>
        (modelDisplayMap.get(a) || a).localeCompare(modelDisplayMap.get(b) || b, undefined, {
          sensitivity: 'base',
        }),
      );
    }

    const fixedModelSet = new Set(defaultGroups.flatMap(group => group.models));
    const otherModels = visibleModels
      .filter(model => !fixedModelSet.has(model.id))
      .sort((a, b) =>
        (a.displayName || a.id).localeCompare(b.displayName || b.id, undefined, {
          sensitivity: 'base',
        }),
      )
      .map(model => model.id);

    const groupsWithOther = [...defaultGroups];
    if (otherModels.length > 0) {
      groupsWithOther.push({
        id: 'other',
        name: t('group_settings.other_group', '其他模型'),
        models: otherModels,
      });
    }

    return groupsWithOther.map(group => ({
      ...group,
      name: savedGroupNames?.[group.id] || group.name,
    }));
  }, [t, visibleModels]);

  // 加载分组配置（合并已保存的名称）
  const loadSettings = useCallback(async () => {
    try {
      const data = await getGroupSettings();
      setGroups(buildGroups(data.groupNames));
      setError(null);
    } catch (err) {
      console.error('Failed to load group settings:', err);
      // 加载失败时使用默认配置
      setGroups(buildGroups());
      setError(null);
    }
  }, [buildGroups]);

  useEffect(() => {
    if (isOpen) {
      loadSettings();
    }
  }, [isOpen, loadSettings]);

  // 修改分组名称
  const handleGroupNameChange = (groupId: string, newName: string) => {
    setGroups(groups.map(g => 
      g.id === groupId ? { ...g, name: newName } : g
    ));
  };

  // 保存配置
  const handleSave = async () => {
    setSaving(true);
    setError(null);
    
    try {
      const groupMappings: Record<string, string> = {};
      const groupNames: Record<string, string> = {};
      const groupOrder: string[] = [];
      
      for (const group of groups) {
        groupOrder.push(group.id);
        groupNames[group.id] = group.name;
        for (const modelId of group.models) {
          groupMappings[modelId] = group.id;
        }
      }
      
      await saveGroupSettings(groupMappings, groupNames, groupOrder);
      onClose();
    } catch (err) {
      console.error('Failed to save group settings:', err);
      setError(String(err));
    } finally {
      setSaving(false);
    }
  };

  if (!isOpen) return null;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal group-settings-modal" onClick={e => e.stopPropagation()}>
        <div className="modal-header">
          <h2>
            <Package size={20} />
            {t('group_settings.title', '分组管理')}
          </h2>
          <button className="modal-close" onClick={onClose}>
            <X size={20} />
          </button>
        </div>
        
        <div className="modal-body">
          {/* 错误提示 */}
          {error && (
            <div className="group-settings-error">
              {error}
            </div>
          )}
          
          {/* 分组列表 */}
          <div className="group-list-section">
            <h3>
              <Package size={16} />
              {t('group_settings.group_list', '分组列表')}
            </h3>
            
            <div className="group-list">
              {groups.map(group => (
                <div key={group.id} className="group-item">
                  <div className="group-header">
                    <Package size={16} className="group-icon" />
                    <input
                      type="text"
                      value={group.name}
                      onChange={e => handleGroupNameChange(group.id, e.target.value)}
                      onKeyDown={e => {
                        if (e.key === 'Enter') {
                          handleSave();
                        }
                      }}
                      className="group-name-input"
                      placeholder={t('group_settings.group_name_placeholder', '分组名称')}
                    />
                  </div>
                  
                  <div className="group-models">
                    {group.models.map(modelId => (
                      <span key={modelId} className="model-tag readonly" title={modelId}>
                        {modelDisplayNameMap.get(modelId) || getModelDisplayName(modelId)}
                      </span>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          </div>
        </div>
        
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose} disabled={saving}>
            {t('common.cancel', '取消')}
          </button>
          <button className="btn btn-primary" onClick={handleSave} disabled={saving}>
            {saving ? t('common.saving', '保存中...') : t('group_settings.save', '保存分组')}
          </button>
        </div>
      </div>
    </div>
  );
};

export default GroupSettingsModal;
