import { create } from "zustand";
import type {
  WindowsOperationErrorDetail,
  WindowsOperationKind,
} from "../utils/windowsOperationError";

export interface WindowsOperationDialogRequest {
  error: WindowsOperationErrorDetail;
  retry?: () => void | Promise<void>;
  manualContinue?: () => void | Promise<void>;
  authorize?: () => void | Promise<void>;
  openTarget?: () => void | Promise<void>;
  onResolved?: () => void | Promise<void>;
}

interface WindowsOperationDialogState {
  request: WindowsOperationDialogRequest | null;
  open: (request: WindowsOperationDialogRequest) => void;
  close: () => void;
  replaceError: (error: WindowsOperationErrorDetail) => void;
}

export const useWindowsOperationDialogStore =
  create<WindowsOperationDialogState>((set) => ({
    request: null,
    open: (request) => set({ request }),
    close: () => set({ request: null }),
    replaceError: (error) =>
      set((state) =>
        state.request
          ? { request: { ...state.request, error } }
          : state,
      ),
  }));

export interface PresentWindowsOperationErrorOptions {
  error: unknown;
  operation?: WindowsOperationKind;
  target?: string | null;
  summary?: string;
  retry?: () => void | Promise<void>;
  manualContinue?: () => void | Promise<void>;
  openTarget?: () => void | Promise<void>;
  onResolved?: () => void | Promise<void>;
}
