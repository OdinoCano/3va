// 11 - React Native: demo ejecutable.
// 3va corre y testea la lógica compartida de tu app React Native
// (los módulos TypeScript puros, sin bridge nativo).
//
//   cd examples/11-react-native
//   3va run src/index.ts
//   3va test

import {
  todoReducer,
  selectProgress,
  prettyProgress,
  selectDueSoon,
  type Todo,
} from "./todos";
import { validateSignup, type SignupInput } from "./validate";

console.log("=== Lógica compartida React Native, ejecutada con 3va ===\n");

let todos: Todo[] = [];

const actions = [
  { type: "add", text: "Revisar el PR de auth" },
  { type: "add", text: "   " },
  { type: "add", text: "Diseñar el onboarding" },
  { type: "add", text: "Actualizar iconos" },
] as const;

for (const a of actions) todos = todoReducer(todos, a);

// Marcar una como hecha
const toggled = todos[0];
todos = todoReducer(todos, { type: "toggle", id: toggled.id });

console.log(`Tareas: ${todos.length}`);
todos.forEach((t) => console.log(`  [${t.done ? "x" : " "}] ${t.text}`));
console.log(`Progreso: ${prettyProgress(todos)}`);

const dueSoon = selectDueSoon(todos, 60_000);
console.log(`Creadas en el último minuto: ${dueSoon.length}`);

// Validación de formulario
const bad: SignupInput = {
  email: "not-an-email",
  password: "short",
  confirmPassword: "different",
};
console.log("\nErrores de validación del formulario de registro:");
validateSignup(bad).forEach((e) => console.log(`  ${e.field}: ${e.message}`));