import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { usePlatformRuntimeSupport } from "./usePlatformRuntimeSupport";

type GeneralConfig = {
  default_terminal: string;
};

export interface LaunchTerminalOption {
  value: string;
  label: string;
}

export function useLaunchTerminalOptions(enabled = true) {
  const { t } = useTranslation();
  const isMacOS = usePlatformRuntimeSupport("macos-only");
  const isWindows = usePlatformRuntimeSupport("windows-only");
  const isLinux = usePlatformRuntimeSupport("linux-only");
  const [availableTerminals, setAvailableTerminals] = useState<string[]>([
    "system",
  ]);
  const [selectedTerminal, setSelectedTerminal] = useState("system");

  useEffect(() => {
    if (!enabled) {
      setAvailableTerminals(["system"]);
      setSelectedTerminal("system");
      return;
    }

    let disposed = false;

    Promise.all([
      invoke<string[]>("get_available_terminals"),
      invoke<GeneralConfig>("get_general_config"),
    ])
      .then(([terminals, config]) => {
        if (disposed) return;
        setAvailableTerminals(
          Array.isArray(terminals) && terminals.length > 0
            ? terminals
            : ["system"],
        );
        setSelectedTerminal(config.default_terminal || "system");
      })
      .catch(() => {
        if (disposed) return;
        setAvailableTerminals(["system"]);
        setSelectedTerminal("system");
      });

    return () => {
      disposed = true;
    };
  }, [enabled]);

  const terminalOptions = useMemo<LaunchTerminalOption[]>(() => {
    const common = [
      {
        value: "system",
        label: t("settings.general.terminalSystem", "跟随系统"),
      },
    ];

    const allOptions = isMacOS
      ? [
          { value: "Terminal", label: "Terminal.app" },
          { value: "iTerm2", label: "iTerm2" },
        ]
      : isWindows
        ? [
            { value: "cmd", label: "Command Prompt (cmd)" },
            { value: "PowerShell", label: "PowerShell" },
            { value: "pwsh", label: "PowerShell Core (pwsh)" },
            { value: "wt", label: "Windows Terminal (wt)" },
          ]
        : isLinux
          ? [
              { value: "x-terminal-emulator", label: "x-terminal-emulator" },
              { value: "gnome-terminal", label: "gnome-terminal" },
              { value: "konsole", label: "konsole" },
              { value: "xfce4-terminal", label: "xfce4-terminal" },
              { value: "xterm", label: "xterm" },
              { value: "alacritty", label: "Alacritty" },
              { value: "kitty", label: "Kitty" },
            ]
          : [];

    return [
      ...common,
      ...allOptions.filter((option) =>
        availableTerminals.includes(option.value),
      ),
    ];
  }, [availableTerminals, isLinux, isMacOS, isWindows, t]);

  useEffect(() => {
    if (terminalOptions.some((option) => option.value === selectedTerminal)) {
      return;
    }
    setSelectedTerminal(terminalOptions[0]?.value ?? "system");
  }, [selectedTerminal, terminalOptions]);

  return {
    terminalOptions,
    selectedTerminal,
    setSelectedTerminal,
  };
}
