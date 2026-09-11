// 11 - React Native: la lógica compartida de la app.
// Módulo TypeScript puro — el mismo código que un equipo React Native
// comparte entre la app (nativa) y el backend. Sin dependencias nativas:
// 3va lo ejecuta y lo testea directamente.

export interface Todo {
  id: string;
  text: string;
  done: boolean;
  createdAt: number;
}

export type TodoAction =
  | { type: "add"; text: string }
  | { type: "toggle"; id: string }
  | { type: "remove"; id: string }
  | { type: "clear-done" };

export function todoReducer(state: Todo[], action: TodoAction): Todo[] {
  switch (action.type) {
    case "add": {
      const text = action.text.trim();
      if (!text) return state;
      return [
        ...state,
        { id: newId(), text, done: false, createdAt: Date.now() },
      ];
    }
    case "toggle":
      return state.map((t) =>
        t.id === action.id ? { ...t, done: !t.done } : t
      );
    case "remove":
      return state.filter((t) => t.id !== action.id);
    case "clear-done":
      return state.filter((t) => !t.done);
  }
}

export function selectOpen(state: Todo[]): Todo[] {
  return state.filter((t) => !t.done);
}

export function selectDone(state: Todo[]): Todo[] {
  return state.filter((t) => t.done);
}

export function selectProgress(state: Todo[]): number {
  if (state.length === 0) return 0;
  return Math.round((selectDone(state).length / state.length) * 100);
}

export function selectDueSoon(state: Todo[], withinMs: number): Todo[] {
  const now = Date.now();
  return state.filter((t) => t.createdAt >= now - withinMs);
}

export function prettyProgress(state: Todo[]): string {
  const pct = selectProgress(state);
  const bar = "█".repeat(Math.round(pct / 10)).padEnd(10, "░");
  return `${bar} ${pct}% (${selectDone(state).length}/${state.length})`;
}

let seq = 0;
function newId(): string {
  seq += 1;
  return `todo-${Date.now().toString(36)}-${seq}`;
}