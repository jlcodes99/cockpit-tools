import { createTraeInstanceService } from '../services/traeInstanceService';
import type { TraePlatformId } from '../services/traeService';
import { createInstanceStore } from './createInstanceStore';

const TRAE_INSTANCE_CACHE_KEYS: Record<TraePlatformId, string> = {
  trae: 'agtools.trae.instances.cache',
  trae_solo: 'agtools.trae_solo.instances.cache',
  trae_cn: 'agtools.trae_cn.instances.cache',
  trae_solo_cn: 'agtools.trae_solo_cn.instances.cache',
};

const createTraeInstanceStoreForPlatform = (platformId: TraePlatformId) =>
  createInstanceStore(
    createTraeInstanceService(platformId),
    TRAE_INSTANCE_CACHE_KEYS[platformId],
  );

export const useTraeInstanceStore = createInstanceStore(
  createTraeInstanceService('trae'),
  TRAE_INSTANCE_CACHE_KEYS.trae,
);

export const useTraeSoloInstanceStore = createTraeInstanceStoreForPlatform('trae_solo');
export const useTraeCnInstanceStore = createTraeInstanceStoreForPlatform('trae_cn');
export const useTraeSoloCnInstanceStore = createTraeInstanceStoreForPlatform('trae_solo_cn');

export const TRAE_INSTANCE_STORES = {
  trae: useTraeInstanceStore,
  trae_solo: useTraeSoloInstanceStore,
  trae_cn: useTraeCnInstanceStore,
  trae_solo_cn: useTraeSoloCnInstanceStore,
} satisfies Record<TraePlatformId, typeof useTraeInstanceStore>;
