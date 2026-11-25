import * as t3codeInstanceService from '../services/t3codeInstanceService';
import { createInstanceStore } from './createInstanceStore';

export const useT3codeInstanceStore = createInstanceStore(
  t3codeInstanceService,
  'agtools.t3code.instances.cache',
);
