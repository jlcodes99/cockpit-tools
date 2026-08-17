import { useEffect, type ReactNode } from "react";
import { setBootSplashStage } from "../utils/bootSplash";

export function BootReadyMarker() {
  useEffect(() => {
    setBootSplashStage("page_ready");
  }, []);
  return null;
}

export function VisibleBootPage({
  when,
  children,
}: {
  when: boolean;
  children: ReactNode;
}) {
  if (!when) {
    return null;
  }
  return (
    <>
      {children}
      <BootReadyMarker />
    </>
  );
}
