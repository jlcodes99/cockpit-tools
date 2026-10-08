import type { InstanceProfile } from "../types/instance";

/** 「获取本地账号」弹框里的单个实例候选。 */
export type CodexLocalImportInstanceOption = {
  id: string;
  /** 实例名；默认实例名称为空，由界面回退为「默认实例」。 */
  name: string;
  userDataDir: string;
  running: boolean;
  isDefault: boolean;
};

/**
 * 构建「获取本地账号」的实例候选列表。
 *
 * 官方客户端按 `CODEX_HOME` 分别落盘凭据，多开实例的本地账号只存在于各自 profile
 * 目录里，因此选择实例即选择要读取的 profile。默认实例始终排在最前，其余实例按
 * 名称排序，避免候选顺序随实例创建时间漂移。
 */
export function buildCodexLocalImportInstanceOptions(
  instances: InstanceProfile[],
): CodexLocalImportInstanceOption[] {
  return instances
    .filter((instance) => Boolean(instance?.id))
    .map((instance) => ({
      id: instance.id,
      name: instance.name ?? "",
      userDataDir: instance.userDataDir ?? "",
      running: Boolean(instance.running),
      isDefault: Boolean(instance.isDefault),
    }))
    .sort((left, right) => {
      if (left.isDefault !== right.isDefault) {
        return left.isDefault ? -1 : 1;
      }
      return (left.name || left.id).localeCompare(right.name || right.id);
    });
}
