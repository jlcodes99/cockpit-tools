import { filterLogContent, type LogContentFilterRequest } from '../utils/logContentFilter';

self.onmessage = (event: MessageEvent<LogContentFilterRequest>) => {
  self.postMessage(filterLogContent(event.data));
};
