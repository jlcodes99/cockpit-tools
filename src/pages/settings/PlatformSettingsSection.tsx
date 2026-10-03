import type { ReactNode } from 'react';

import { usePlatformLayoutStore } from '../../stores/usePlatformLayoutStore';
import type { PlatformId } from '../../types/platform';

/**
 * 平台设置分区容器。
 *
 * 平台在「平台布局」中被禁用时整个分区不再渲染，避免用户去配置一个
 * 后台已经完全停摆的平台。顺序仍由调用方传入的 order 控制，与平台布局
 * 中的自定义排序保持一致。
 */
export function PlatformSettingsSection({
  platformId,
  order,
  children,
}: {
  platformId: PlatformId;
  order: number;
  children: ReactNode;
}) {
  const disabled = usePlatformLayoutStore((state) =>
    state.disabledPlatformIds.includes(platformId),
  );

  if (disabled) {
    return null;
  }

  return <div style={{ order }}>{children}</div>;
}
