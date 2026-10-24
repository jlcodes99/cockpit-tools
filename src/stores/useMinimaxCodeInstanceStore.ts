import { createInstanceStore } from './createInstanceStore';
import * as minimaxCodeInstanceService from '../services/minimaxCodeInstanceService';

export const useMinimaxCodeInstanceStore = createInstanceStore(
  minimaxCodeInstanceService,
  'agtools.minimaxcode.instances.cache',
);
