// 13 - Tauri: el frontend (main.ts).
// 3va dev lo sirve transpilando .ts por request; 3va bundle produce un
// dist/bundle.js listo para incrustar en src-tauri/dist.

import { applyCommand, initialAppState, type AppState, type ThemeChange, type VolumeChange } from "./state.ts";

const app = document.getElementById("app")!;

let state: AppState = initialAppState();

function render() {
  const themeBtn = state.theme === "light" ? "🌙 dark" : "☀️ light";
  app.innerHTML = `
    <div style="max-width:420px;margin:0 auto;font-family:system-ui">
      <h2>Audio (simulando comandos Tauri)</h2>
      <p>Volumen: <strong>${state.volume}</strong>%
        ${state.muted ? " <em>(muted)</em>" : ""}</p>
      <input id="vol" type="range" min="0" max="100" value="${state.volume}" />
      <button id="theme">${themeBtn}</button>
      <button id="mute">${state.muted ? "Unmute" : "Mute"}</button>
    </div>
  `;

  const dispatch = (cmd: ThemeChange | VolumeChange) => {
    const res = applyCommand(state, cmd);
    if (res.status === "ok") state = res.state;
    render();
  };

  document.getElementById("vol")!.addEventListener("input", (e) => {
    dispatch({ kind: "set-volume", value: Number((e.target as HTMLInputElement).value) });
  });
  document.getElementById("theme")!.addEventListener("click", () => {
    dispatch({ kind: "set-theme", theme: state.theme === "light" ? "dark" : "light" });
  });
  document.getElementById("mute")!.addEventListener("click", () => {
    dispatch(state.muted ? { kind: "unmute" } : { kind: "mute" });
  });
}

render();
console.log("3va + Tauri frontend mounted");