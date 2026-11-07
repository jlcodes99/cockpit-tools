import { useRef, type RefObject } from 'react';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';

interface WakeupModalVisibility {
  task: boolean;
  test: boolean;
  history: boolean;
  results: boolean;
  preset: boolean;
  runtime: boolean;
}

/** Child dialogs suspend their parent trap without losing the parent's return target. */
export function useWakeupModalFocus(open: WakeupModalVisibility, task: RefObject<HTMLDivElement | null>) {
  const test = useRef<HTMLDivElement>(null);
  const history = useRef<HTMLDivElement>(null);
  const results = useRef<HTMLDivElement>(null);
  const preset = useRef<HTMLDivElement>(null);
  const runtime = useRef<HTMLDivElement>(null);
  const top = open.runtime ? 'runtime' : open.preset ? 'preset' : open.results ? 'results'
    : open.history ? 'history' : open.test ? 'test' : 'task';
  useModalFocusTrap(task, open.task, true, top === 'task');
  useModalFocusTrap(test, open.test, true, top === 'test');
  useModalFocusTrap(history, open.history, true, top === 'history');
  useModalFocusTrap(results, open.results, true, top === 'results');
  useModalFocusTrap(preset, open.preset, true, top === 'preset');
  useModalFocusTrap(runtime, open.runtime, true, top === 'runtime');
  return { task, test, history, results, preset, runtime, top };
}
