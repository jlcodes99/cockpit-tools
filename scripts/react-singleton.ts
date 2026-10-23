import fs from "node:fs";
import type { Plugin } from "vite";

function reactRuntimePath(id: string): string | null {
  // CommonJS proxy/wrapper modules refer to a runtime; they do not instantiate
  // another one. Real modules with distinct IDs, even aliases of one physical
  // file, can each instantiate React and must not be collapsed by realpath.
  if (id.startsWith("\0")) return null;
  try {
    const canonical = fs.realpathSync.native(id.split("?", 1)[0]);
    return /[\\/]node_modules[\\/]react[\\/]cjs[\\/]react\.(?:production|development)\.js$/.test(canonical)
      ? canonical
      : null;
  } catch {
    return null;
  }
}

export function assertReactSingleton(): Plugin {
  return {
    name: "assert-react-singleton",
    apply: "build",
    generateBundle() {
      const runtimes = new Map<string, string>();
      for (const id of this.getModuleIds()) {
        if (!this.getModuleInfo(id)?.isIncluded) continue;
        const canonical = reactRuntimePath(id);
        if (canonical) runtimes.set(id, canonical);
      }
      if (runtimes.size > 1) {
        this.error(`Multiple React runtime module IDs in frontend bundle:\n${
          [...runtimes].map(([id, canonical]) => `${id} -> ${canonical}`).join("\n")
        }`);
      }
    },
  };
}
