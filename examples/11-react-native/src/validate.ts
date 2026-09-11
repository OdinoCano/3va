// 11 - React Native: validaciones de formulario compartidas.
// Típico de una app RN: validar en el cliente ANTES de la petición al backend.

export interface FieldError {
  field: string;
  message: string;
}

export interface SignupInput {
  email: string;
  password: string;
  confirmPassword: string;
}

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

export function validateEmail(email: string): string | null {
  if (!email.trim()) return "Email requerido";
  if (email.length > 254) return "Email demasiado largo";
  if (!EMAIL_RE.test(email)) return "Email no válido";
  return null;
}

export function validatePassword(password: string): string | null {
  if (password.length < 8) return "Mínimo 8 caracteres";
  if (!/[A-Z]/.test(password)) return "Falta una mayúscula";
  if (!/[0-9]/.test(password)) return "Falta un número";
  return null;
}

export function validateSignup(input: SignupInput): FieldError[] {
  const errors: FieldError[] = [];
  const email = validateEmail(input.email);
  if (email) errors.push({ field: "email", message: email });
  const password = validatePassword(input.password);
  if (password) errors.push({ field: "password", message: password });
  if (input.confirmPassword !== input.password) {
    errors.push({ field: "confirmPassword", message: "Las contraseñas no coinciden" });
  }
  return errors;
}

export function hasErrors(input: SignupInput): boolean {
  return validateSignup(input).length > 0;
}