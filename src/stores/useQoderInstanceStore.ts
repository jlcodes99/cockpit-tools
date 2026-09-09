import { createQoderInstanceService } from '../services/qoderInstanceService';
import type { QoderPlatformId } from '../types/qoder';
import { createInstanceStore } from './createInstanceStore';

const QODER_INSTANCE_CACHE_KEYS: Record<QoderPlatformId, string> = {
  qoder: 'agtools.qoder.instances.cache',
  qoder_app: 'agtools.qoder_app.instances.cache',
  qoder_cn_ide: 'agtools.qoder_cn_ide.instances.cache',
  qoder_cn_app: 'agtools.qoder_cn_app.instances.cache',
};

const createQoderInstanceStoreForPlatform = (platformId: QoderPlatformId) =>
  createInstanceStore(
    createQoderInstanceService(platformId),
    QODER_INSTANCE_CACHE_KEYS[platformId],
  );

export const useQoderInstanceStore = createInstanceStore(
  createQoderInstanceService('qoder'),
  QODER_INSTANCE_CACHE_KEYS.qoder,
);

export const useQoderAppInstanceStore = createQoderInstanceStoreForPlatform('qoder_app');
export const useQoderCnIdeInstanceStore = createQoderInstanceStoreForPlatform('qoder_cn_ide');
export const useQoderCnAppInstanceStore = createQoderInstanceStoreForPlatform('qoder_cn_app');

export const QODER_INSTANCE_STORES = {
  qoder: useQoderInstanceStore,
  qoder_app: useQoderAppInstanceStore,
  qoder_cn_ide: useQoderCnIdeInstanceStore,
  qoder_cn_app: useQoderCnAppInstanceStore,
} satisfies Record<QoderPlatformId, typeof useQoderInstanceStore>;
