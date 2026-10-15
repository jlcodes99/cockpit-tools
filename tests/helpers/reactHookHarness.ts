import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';

/** Minimal hook scheduler for exercising source handlers without a GUI. */
export function loadHookModule(file: URL, imports: Record<string, unknown>, globals: Record<string, unknown> = {}) {
  const values: any[] = [];
  const dependencies: (unknown[] | undefined)[] = [];
  const cleanups: (((() => void)) | undefined)[] = [];
  const effects: (() => void)[] = [];
  let cursor = 0;
  let dirty = false;
  let renderCurrent: (() => any) | undefined;
  const same = (a?: unknown[], b?: unknown[]) => !!a && !!b && a.length === b.length && a.every((value, index) => Object.is(value, b[index]));
  const hooks = {
    useState(initial: any) {
      const index = cursor++;
      if (!(index in values)) values[index] = typeof initial === 'function' ? initial() : initial;
      return [values[index], (next: any) => {
        const value = typeof next === 'function' ? next(values[index]) : next;
        if (!Object.is(value, values[index])) { values[index] = value; dirty = true; }
      }];
    },
    useRef(initial: unknown) { const index = cursor++; return values[index] ??= { current: initial }; },
    useId() { const index = cursor++; return values[index] ??= `hook-id-${index}`; },
    useEffect(effect: () => void | (() => void), deps?: unknown[]) {
      const index = cursor++;
      if (same(dependencies[index], deps)) return;
      dependencies[index] = deps;
      effects.push(() => { cleanups[index]?.(); cleanups[index] = effect() || undefined; });
    },
    useLayoutEffect(effect: () => void | (() => void), deps?: unknown[]) { hooks.useEffect(effect, deps); },
    useMemo(factory: () => unknown, deps?: unknown[]) {
      const index = cursor++;
      if (!same(dependencies[index], deps)) { values[index] = factory(); dependencies[index] = deps; }
      return values[index];
    },
    useCallback(callback: unknown, deps?: unknown[]) { return hooks.useMemo(() => callback, deps); },
  };
  const exports: Record<string, any> = {};
  const require = createRequire(file);
  // Context creation is not a hook; retain React's provider shape for context-owner tests.
  const createContext = require('react').createContext;
  const compiled = ts.transpileModule(readFileSync(file, 'utf8'), { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX,
  } }).outputText;
  vm.runInNewContext(compiled, { exports, crypto: globalThis.crypto, ...globals, require(name: string) {
    if (name === 'react') return { ...hooks, createContext };
    if (name in imports) return imports[name];
    if (name.endsWith('.css')) return {};
    return require(name);
  } });
  function flush(): any {
    if (!renderCurrent) throw new Error('Render the hook before flushing');
    let output: any;
    let remaining = 50;
    do {
      if (!remaining--) throw new Error('Hook did not settle');
      dirty = false; cursor = 0; output = renderCurrent();
      while (effects.length) effects.shift()!();
    } while (dirty);
    return output;
  }
  return { exports, flush, render(callback: () => any) { renderCurrent = callback; return flush(); },
    unmount() { cleanups.forEach((cleanup) => cleanup?.()); },
  };
}

export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

export const settlePromises = () => new Promise<void>((resolve) => setImmediate(resolve));
