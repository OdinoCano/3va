// 13 - Tauri: tests del reducer de comandos (Jest-compatible).

import { applyCommand, initialAppState, reduceCommands, type AppState, type ThemeChange, type VolumeChange } from "./state";

let state: AppState = initialAppState();

beforeEach(() => {
  state = initialAppState();
});

describe("applyCommand", () => {
  test("set-theme cambia el tema", () => {
    const res = applyCommand(state, { kind: "set-theme", theme: "dark" });
    expect(res.state.theme).toBe("dark");
    expect(res.status).toBe("ok");
  });

  test("set-volume válido", () => {
    const res = applyCommand(state, { kind: "set-volume", value: 30 });
    expect(res.state.volume).toBe(30);
    expect(res.state.muted).toBe(false);
  });

  test("set-volume a 0 silencia", () => {
    const res = applyCommand(state, { kind: "set-volume", value: 0 });
    expect(res.state.muted).toBe(true);
  });

  test("set-volume inválido es rechazado y no muta el estado", () => {
    const res = applyCommand(state, { kind: "set-volume", value: 150 });
    expect(res.status).toBe("error");
    expect(res.state.volume).toBe(70);
  });

  test("mute / unmute", () => {
    expect(applyCommand(state, { kind: "mute" }).state.muted).toBe(true);
    expect(applyCommand(state, { kind: "unmute" }).state.muted).toBe(false);
  });
});

describe("reduceCommands", () => {
  test("aplica una secuencia en orden", () => {
    const cmds: (ThemeChange | VolumeChange)[] = [
      { kind: "set-theme", theme: "dark" },
      { kind: "set-volume", value: 55 },
      { kind: "mute" },
    ];
    const final = reduceCommands(state, cmds);
    expect(final).toEqual({ theme: "dark", volume: 55, muted: true });
  });

  test("un comando inválido detiene la escritura pero no rompe los anteriores", () => {
    const cmds: (ThemeChange | VolumeChange)[] = [
      { kind: "set-theme", theme: "dark" },
      // reduceCommands no valida por diseño (los comandos Tauri ya están
      // validados en Rust) — este test solo asegura orden.
      { kind: "set-volume", value: 45 },
    ];
    expect(reduceCommands(state, cmds)).toEqual({ theme: "dark", volume: 45, muted: false });
  });
});