import * as piInstanceService from '../services/piInstanceService';
import { createInstanceStore } from './createInstanceStore';

export const usePiInstanceStore = createInstanceStore(
  piInstanceService,
  'agtools.pi.instances.cache',
);
