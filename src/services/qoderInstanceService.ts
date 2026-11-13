import { formatQoderError } from '../utils/qoderErrors';
import { createPlatformInstanceService } from './platform/createPlatformInstanceService';

const service = createPlatformInstanceService('qoder');

export const getInstanceDefaults = service.getInstanceDefaults;
export const listInstances = service.listInstances;
export const createInstance = withQoderError(service.createInstance);
export const updateInstance = withQoderError(service.updateInstance);
export const deleteInstance = withQoderError(service.deleteInstance);
export const startInstance = withQoderError(service.startInstance);
export const stopInstance = withQoderError(service.stopInstance);
export const closeAllInstances = withQoderError(service.closeAllInstances);
export const openInstanceWindow = withQoderError(service.openInstanceWindow);

function withQoderError<Args extends unknown[], Result>(action: (...args: Args) => Promise<Result>) {
  return async (...args: Args): Promise<Result> => {
    try {
      return await action(...args);
    } catch (error) {
      throw formatQoderError(error);
    }
  };
}
