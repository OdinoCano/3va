// 11 - React Native: tests de la lógica compartida (Jest-compatible).

import {
  todoReducer,
  selectProgress,
  prettyProgress,
  type Todo,
} from "./todos";
import { validateSignup, validateEmail, validatePassword } from "./validate";

const START: Todo[] = [
  { id: "1", text: "Lear", done: false, createdAt: 100 },
  { id: "2", text: "Discipulado", done: true, createdAt: 200 },
];

describe("todoReducer", () => {
  test("agrega una tarea limpia (trim) y asigna id", () => {
    const state = todoReducer(START, { type: "add", text: "  Hola  " });
    expect(state).toHaveLength(3);
    expect(state[2].text).toBe("Hola");
    expect(state[2].done).toBe(false);
    expect(state[2].id).toBeTruthy();
  });

  test("ignora tareas vacías", () => {
    expect(todoReducer(START, { type: "add", text: "   " })).toHaveLength(2);
  });

  test("toggle marca/desmarca una tarea", () => {
    const s = todoReducer(START, { type: "toggle", id: "1" });
    expect(s[0].done).toBe(true);
    expect(s[1].done).toBe(true); // la otra no cambia
  });

  test("remove elimina la tarea exacta", () => {
    const s = todoReducer(START, { type: "remove", id: "1" });
    expect(s.map((t) => t.id)).toEqual(["2"]);
  });

  test("clear-done elimina solo las completadas", () => {
    const s = todoReducer(START, { type: "clear-done" });
    expect(s).toHaveLength(1);
    expect(s[0].id).toBe("1");
  });
});

describe("selectProgress / prettyProgress", () => {
  test("progreso 50% con una de dos hecha", () => {
    expect(selectProgress(START)).toBe(50);
  });

  test("prettyProgress dibuja la barra y el conteo", () => {
    expect(prettyProgress(START)).toMatch(/50% \(1\/2\)$/);
  });
});

describe("validateSignup", () => {
  test("email inválido", () => {
    expect(validateEmail("nope")).toBe("Email no válido");
    expect(validateEmail("a@b.co")).toBeNull();
  });

  test("password débil", () => {
    expect(validatePassword("12345678")).toBe("Falta una mayúscula");
    expect(validatePassword("Hola1234")).toBeNull();
  });

  test("contraseñas que no coinciden", () => {
    const errors = validateSignup({
      email: "a@b.co",
      password: "Hola1234",
      confirmPassword: "Otro1234",
    });
    expect(errors.map((e) => e.field)).toContain("confirmPassword");
  });
});