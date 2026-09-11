// 13 - Tauri: el "backend" de comandos del frontend.
// En Tauri, la lógica de estado vive en Rust (#[tauri::command]). El frontend
// la llama por IPC con tipos compartidos. Aquí la misma firma tipada corre en
// TypeScript puro, de modo que 3va la testea sin compilar el shell Rust.

export interface AppState {
  theme: "light" | "dark";
  volume: number; // 0..100
  muted: boolean;
}

export function initialAppState(): AppState {
  return { theme: "light", volume: 70, muted: false };
}

export type ThemeChange = { kind: "set-theme"; theme: AppState["theme"] };
export type VolumeChange =
  | { kind: "set-volume"; value: number }
  | { kind: "mute" }
  | { kind: "unmute" };

export interface CommandResult {
  state: AppState;
  status: "ok" | "error";
  message: string;
}

export function applyCommand(
  state: AppState,
  cmd: ThemeChange | VolumeChange
): CommandResult {
  switch (cmd.kind) {
    case "set-theme":
      return { state: { ...state, theme: cmd.theme }, status: "ok", message: "theme updated" };
    case "set-volume": {
      if (!Number.isFinite(cmd.value) || cmd.value < 0 || cmd.value > 100) {
        return { state, status: "error", message: "volume must be 0..100" };
      }
      return { state: { ...state, volume: cmd.value, muted: cmd.value === 0 }, status: "ok", message: "volume updated" };
    }
    case "mute":
      return { state: { ...state, muted: true }, status: "ok", message: "muted" };
    case "unmute":
      return { state: { ...state, muted: false }, status: "ok", message: "unmuted" };
  }
}

/** Reducer de comandos encolados — el front lo despacha por IPC. */
export function reduceCommands(initial: AppState, cmds: (ThemeChange | VolumeChange)[]): AppState {
  return cmds.reduce((s, c) => applyCommand(s, c).state, initial);
}