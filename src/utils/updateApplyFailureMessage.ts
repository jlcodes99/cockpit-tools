import type { TFunction } from 'i18next';

export type UpdateApplyFailureStage = 'prepare' | 'install' | 'relaunch';

export function resolveUpdateApplyFailureMessage(
  t: TFunction,
  stage: UpdateApplyFailureStage,
  isTranslocated: boolean,
): string {
  if (stage === 'prepare') {
    return t('update_notification.stopApiServiceFailed', '无法关闭 API 服务，请先停用后重试。');
  }
  if (stage === 'relaunch') {
    return t('update_notification.restartRequiredAfterInstall', '更新已安装，请手动重启应用完成切换。');
  }
  // A translocated app runs from a read-only mount, so retrying the install can never succeed.
  if (isTranslocated) {
    return t(
      'update_notification.installFailedTranslocated',
      '应用正在只读位置运行（macOS App Translocation），无法安装更新。请退出 Cockpit Tools，在访达中将其移动到“应用程序”文件夹，重新打开后再重试更新。',
    );
  }
  return t('update_notification.installFailed', '系统安装失败，请稍后重试或手动下载安装。');
}
